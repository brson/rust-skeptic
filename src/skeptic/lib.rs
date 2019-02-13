extern crate pulldown_cmark as cmark;
extern crate tempdir;
extern crate glob;
extern crate bytecount;
extern crate toml;

use std::env;
use std::error::Error as StdError;
use std::path::{PathBuf, Path};
use std::collections::{HashMap, BTreeMap};
use toml::Value;

/// Returns a list of markdown files under a directory.
///
/// # Usage
///
/// List markdown files of `mdbook` which are under `<project dir>/book` usually:
///
/// ```rust
/// extern crate skeptic;
///
/// use skeptic::markdown_files_of_directory;
///
/// fn main() {
///     let _ = markdown_files_of_directory("book/");
/// }
/// ```
pub fn markdown_files_of_directory(dir: &str) -> Vec<PathBuf> {
    use glob::{glob_with, MatchOptions};

    let opts = MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let mut out = Vec::new();

    for path in glob_with(&format!("{}/**/*.md", dir), &opts)
        .expect("Failed to read glob pattern")
        .filter_map(Result::ok)
    {
        out.push(path.to_str().unwrap().into());
    }

    out
}

/// Generates tests for specified markdown files.
///
/// # Usage
///
/// Generates doc tests for the specified files.
///
/// ```rust,no_run
/// extern crate skeptic;
///
/// use skeptic::generate_doc_tests;
///
/// fn main() {
///     generate_doc_tests(&["README.md"]);
/// }
/// ```
///
/// Or in case you want to add `mdbook` files:
///
/// ```rust,no_run
/// extern crate skeptic;
///
/// use skeptic::*;
///
/// fn main() {
///     let mut mdbook_files = markdown_files_of_directory("book/");
///     mdbook_files.push("README.md".into());
///     generate_doc_tests(&mdbook_files);
/// }
/// ```
pub fn generate_doc_tests<T: Clone>(docs: &[T])
where
    T: AsRef<Path>,
{
    // This shortcut is specifically so examples in skeptic's own
    // readme can call this function in non-build.rs contexts, without
    // panicking below.
    if docs.is_empty() {
        return;
    }

    let docs = docs.iter()
        .cloned()
        .map(|path| path.as_ref().to_str().unwrap().to_owned())
        .filter(|d| !d.ends_with(".skt.md"))
        .collect::<Vec<_>>();

    // Inform cargo that it needs to rerun the build script if one of the skeptic files are
    // modified
    for doc in &docs {
        println!("cargo:rerun-if-changed={}", doc);

        let skt = format!("{}.skt.md", doc);
        if Path::new(&skt).exists() {
            println!("cargo:rerun-if-changed={}", skt);
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("could not get OUTDIR"));
    let cargo_manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("could_not_get CARGO_MANIFEST_DIR"));
    let target_triple = env::var("TARGET").expect("could not get TARGET");

    // TODO: I'm skeptical of using this value because it is the actual cargo bin,
    // in the toolchain, and not the rustup wrapper.
    // TODO: Should we also use RUSTC_WRAPPER?
    let cargo = env::var("CARGO").expect("could not get CARGO");
    let rustc = env::var("RUSTC").expect("could not get RUSTC");

    let mut test_dir = cargo_manifest_dir.clone();
    test_dir.push("tests/skeptic");
    let mut test_file = test_dir.clone();
    test_file.push("skeptic-tests.rs");

    let (target_dir, out_dir_has_triple) = target_dir_from_out_dir(&out_dir, &target_triple);

    let manifest_info = extract_manifest_info(&cargo_manifest_dir)
        .expect("unable to parse manifest for skeptic test generation");

    let config = Config {
        root_dir: cargo_manifest_dir,
        test_dir: test_dir,
        test_file: test_file,
        target_dir: target_dir,
        target_triple: target_triple,
        out_dir_has_triple: out_dir_has_triple,
        cargo: cargo,
        rustc: rustc,
        docs: docs,
        manifest_info: manifest_info,
    };

    run(&config);
}

/// Derive target_dir from out_dir. Hope this is correct...
///
/// Two cases:
///
/// - $target_dir/(debug|release)/build/$(root_project_name)-$hash/out/
/// - $target_dir/$target_triple/(debug|release)/build/$(root_project_name)-$hash/out/
///
fn target_dir_from_out_dir(out_dir: &Path, target_triple: &str) -> (PathBuf, bool) {

    let mut target_dir = out_dir.to_owned();

    assert!(target_dir.ends_with("out"));
    assert!(target_dir.pop());
    assert!(target_dir.pop());
    assert!(target_dir.ends_with("build"));
    assert!(target_dir.pop());
    assert!(target_dir.ends_with("debug") || target_dir.ends_with("release"));
    assert!(target_dir.pop());

    if target_dir.ends_with(target_triple) {
        assert!(target_dir.pop());
        (target_dir, true)
    } else {
        (target_dir, false)
    }
}

fn extract_manifest_info(manifest_dir: &Path) -> Result<ManifestInfo, Box<StdError + Sync + Send + 'static>> {

    use std::fs::File;
    use std::io::Read;
    use toml::Value;

    let mut manifest = manifest_dir.to_owned();
    manifest.push("Cargo.toml");

    let mut manifest_buf = String::new();

    File::open(manifest)?.read_to_string(&mut manifest_buf)?;

    let mani_value = manifest_buf.parse::<Value>()?;

    let mut deps = None;
    let mut dev_deps = None;
    let mut build_deps = None;

    if let Value::Table(sections) = mani_value {
        for (sec_key, sec_value) in sections {
            match sec_key.as_str() {
                "dependencies" => {
                    deps = Some(sanitize_deps(sec_value));
                }
                "dev-dependencies" => {
                    dev_deps = Some(sanitize_deps(sec_value));
                }
                "build-dependencies" => {
                    build_deps = Some(sanitize_deps(sec_value));
                }
                _ => { }
            }
        }
    } else {
        panic!("unexpected toml type in manifest {:?}", mani_value);
    }

    Ok(ManifestInfo {
        deps, dev_deps, build_deps,
    })
}

fn sanitize_deps(toml: Value) -> Value {
    if let Value::Table(deps) = toml {
        let mut new_deps = BTreeMap::new();

        for (name, props) in deps {
            if let Value::Table(props) = props {
                let mut new_props = BTreeMap::new();

                for (prop_name, prop_value) in props {
                    if prop_name == "path" {
                        if let Value::String(prop_value) = prop_value {
                            let path = PathBuf::from(&prop_value);
                            if !path.is_absolute() {
                                // rewrite dependency paths to account for the location
                                // of the test manifest, "tests/skeptic/$test_name/"
                                // FIXME: This only works 
                                let mut prop_value = format!("../../../{}", prop_value);
                                new_props.insert(prop_name, Value::String(prop_value));
                            } else {
                                new_props.insert(prop_name, Value::String(prop_value));
                            }
                        } else {
                            new_props.insert(prop_name, prop_value);
                        }
                    } else {
                        new_props.insert(prop_name, prop_value);
                    }
                }

                new_deps.insert(name, Value::Table(new_props));
            } else if let Value::String(s) = props {
                new_deps.insert(name, Value::String(s));
            } else {
                panic!("dep props are not a table or string: {:?}", props);
            }
        }

        Value::Table(new_deps)
    } else {
        panic!("deps are not a table");
    }
}

struct Config {
    root_dir: PathBuf,
    test_dir: PathBuf,
    test_file: PathBuf,
    target_dir: PathBuf,
    target_triple: String,
    out_dir_has_triple: bool,
    cargo: String,
    rustc: String,
    docs: Vec<String>,
    manifest_info: ManifestInfo,
}

#[derive(Debug)]
struct ManifestInfo {
    deps: Option<Value>,
    dev_deps: Option<Value>,
    build_deps: Option<Value>,
}

fn run(config: &Config) {
    let tests = extract::extract_tests(config).unwrap();
    emit::emit_tests(config, tests).unwrap();
}

struct Test {
    name: String,
    text: Vec<String>,
    ignore: bool,
    no_run: bool,
    should_panic: bool,
    template: Option<String>,
}

struct DocTestSuite {
    doc_tests: Vec<DocTest>,
}

struct DocTest {
    path: PathBuf,
    old_template: Option<String>,
    tests: Vec<Test>,
    templates: HashMap<String, String>,
}

mod extract {

    use std::fs::File;
    use std::io::{Read, Error as IoError};
    use std::mem;
    use std::path::Path;
    use std::collections::HashMap;
    use super::{Config, Test, DocTest, DocTestSuite};
    use cmark::{Parser, Event, Tag};

    pub (in super) fn extract_tests(config: &Config) -> Result<DocTestSuite, IoError> {
        let mut doc_tests = Vec::new();
        for doc in &config.docs {
            let path = &mut config.root_dir.clone();
            path.push(doc);
            let new_tests = extract_tests_from_file(path)?;
            doc_tests.push(new_tests);
        }
        Ok(DocTestSuite { doc_tests: doc_tests })
    }

    enum Buffer {
        None,
        Code(Vec<String>),
        Header(String),
    }

    fn extract_tests_from_file(path: &Path) -> Result<DocTest, IoError> {
        let mut file = File::open(path)?;
        let s = &mut String::new();
        file.read_to_string(s)?;

        let file_stem = &sanitize_test_name(path.file_stem().unwrap().to_str().unwrap());

        let tests = extract_tests_from_string(s, file_stem);

        let templates = load_templates(path)?;

        Ok(DocTest {
            path: path.to_owned(),
            old_template: tests.1,
            tests: tests.0,
            templates: templates,
        })
    }

    fn extract_tests_from_string(s: &str, file_stem: &str) -> (Vec<Test>, Option<String>) {
        let mut tests = Vec::new();
        let mut buffer = Buffer::None;
        let mut parser = Parser::new(s);
        let mut section = None;
        let mut code_block_start = 0;
        // Oh this isn't actually a test but a legacy template
        let mut old_template = None;

        // In order to call get_offset() on the parser,
        // this loop must not hold an exclusive reference to the parser.
        loop {
            let offset = parser.get_offset();
            let line_number = bytecount::count(&s.as_bytes()[0..offset], b'\n');
            let event = if let Some(event) = parser.next() {
                event
            } else {
                break;
            };
            match event {
                Event::Start(Tag::Header(level)) if level < 3 => {
                    buffer = Buffer::Header(String::new());
                }
                Event::End(Tag::Header(level)) if level < 3 => {
                    let cur_buffer = mem::replace(&mut buffer, Buffer::None);
                    if let Buffer::Header(sect) = cur_buffer {
                        section = Some(sanitize_test_name(&sect));
                    }
                }
                Event::Start(Tag::CodeBlock(ref info)) => {
                    let code_block_info = parse_code_block_info(info);
                    if code_block_info.is_rust {
                        buffer = Buffer::Code(Vec::new());
                    }
                }
                Event::Text(text) => {
                    if let Buffer::Code(ref mut buf) = buffer {
                        if buf.is_empty() {
                            code_block_start = line_number;
                        }
                        buf.push(text.into_owned());
                    } else if let Buffer::Header(ref mut buf) = buffer {
                        buf.push_str(&*text);
                    }
                }
                Event::End(Tag::CodeBlock(ref info)) => {
                    let code_block_info = parse_code_block_info(info);
                    if let Buffer::Code(buf) = mem::replace(&mut buffer, Buffer::None) {
                        if code_block_info.is_old_template {
                            old_template = Some(buf.into_iter().collect())
                        } else {
                            let name = if let Some(ref section) = section {
                                format!("{}_sect_{}_line_{}", file_stem, section, code_block_start)
                            } else {
                                format!("{}_line_{}", file_stem, code_block_start)
                            };
                            tests.push(Test {
                                name: name,
                                text: buf,
                                ignore: code_block_info.ignore,
                                no_run: code_block_info.no_run,
                                should_panic: code_block_info.should_panic,
                                template: code_block_info.template,
                            });
                        }
                    }
                }
                _ => (),
            }
        }
        (tests, old_template)
    }

    fn load_templates(path: &Path) -> Result<HashMap<String, String>, IoError> {
        let file_name = format!(
            "{}.skt.md",
            path.file_name().expect("no file name").to_string_lossy()
        );
        let path = path.with_file_name(&file_name);
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let mut map = HashMap::new();

        let mut file = File::open(path)?;
        let s = &mut String::new();
        file.read_to_string(s)?;
        let parser = Parser::new(s);

        let mut code_buffer = None;

        for event in parser {
            match event {
                Event::Start(Tag::CodeBlock(ref info)) => {
                    let code_block_info = parse_code_block_info(info);
                    if code_block_info.is_rust {
                        code_buffer = Some(Vec::new());
                    }
                }
                Event::Text(text) => {
                    if let Some(ref mut buf) = code_buffer {
                        buf.push(text.to_string());
                    }
                }
                Event::End(Tag::CodeBlock(ref info)) => {
                    let code_block_info = parse_code_block_info(info);
                    if let Some(buf) = code_buffer.take() {
                        if let Some(t) = code_block_info.template {
                            map.insert(t, buf.into_iter().collect());
                        }
                    }
                }
                _ => (),
            }
        }

        Ok(map)
    }

    fn sanitize_test_name(s: &str) -> String {
        s.to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii() && ch.is_alphanumeric() {
                ch
            } else {
                '_'
            })
            .collect::<String>()
            .split('_')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_")
    }

    fn parse_code_block_info(info: &str) -> CodeBlockInfo {
        // Same as rustdoc
        let tokens = info.split(|c: char| !(c == '_' || c == '-' || c.is_alphanumeric()));

        let mut seen_rust_tags = false;
        let mut seen_other_tags = false;
        let mut info = CodeBlockInfo {
            is_rust: false,
            should_panic: false,
            ignore: false,
            no_run: false,
            is_old_template: false,
            template: None,
        };

        for token in tokens {
            match token {
                "" => {}
                "rust" => {
                    info.is_rust = true;
                    seen_rust_tags = true
                }
                "should_panic" => {
                    info.should_panic = true;
                    seen_rust_tags = true
                }
                "ignore" => {
                    info.ignore = true;
                    seen_rust_tags = true
                }
                "no_run" => {
                    info.no_run = true;
                    seen_rust_tags = true;
                }
                "skeptic-template" => {
                    info.is_old_template = true;
                    seen_rust_tags = true
                }
                _ if token.starts_with("skt-") => {
                    info.template = Some(token[4..].to_string());
                    seen_rust_tags = true;
                }
                _ => seen_other_tags = true,
            }
        }

        info.is_rust &= !seen_other_tags || seen_rust_tags;

        info
    }

    struct CodeBlockInfo {
        is_rust: bool,
        should_panic: bool,
        ignore: bool,
        no_run: bool,
        is_old_template: bool,
        template: Option<String>,
    }
}

mod emit {
#![allow(warnings)] // todo

    use std::collections::{BTreeMap, VecDeque};
    use std::error::Error as StdError;
    use std::fmt::Write;
    use std::fs::{self, File};
    use std::io::{self, Read, Error as IoError};
    use std::path::Path;
    use super::{Config, DocTestSuite, DocTest, Test, ManifestInfo};
    use toml::Value;

    pub (in super) fn emit_tests(config: &Config, suite: DocTestSuite) -> Result<(), Box<StdError + Send + Sync + 'static>> {
        emit_test_cases(config, &suite)?;
        emit_test_projects(config, &suite)?;
        emit_supercrate_project(config, &suite)?;
        Ok(())
    }

    fn emit_test_cases(config: &Config, suite: &DocTestSuite) -> Result<(), IoError> {
        let mut buf = String::new();

        writeln!(buf, "use std::process::Command;");
        writeln!(buf);

        for test_doc in &suite.doc_tests {
            for test in &test_doc.tests {
                let mut s = String::new();

                if test.ignore { writeln!(s, "/* skeptic-ignored test"); }

                if test.no_run { writeln!(s, "// skeptic-no_run test"); }

                if test.should_panic { writeln!(s, "#[should_panic]"); }

                writeln!(s, "#[test]");
                writeln!(s, "fn {}() {{", test.name);

                // todo: --release, --nocapture
                
                writeln!(s, r#"    let mut cmd = Command::new("{}");"#, config.cargo);
                writeln!(s, r#"    cmd"#);
                writeln!(s, r#"        .env("RUSTC", "{}")"#, config.rustc);
                // ... shhhhhh ...
                writeln!(s, r#"        .env("RUSTC_BOOTSTRAP", "1")"#);
                if !test.no_run {
                    writeln!(s, r#"        .arg("run")"#);
                } else {
                    writeln!(s, r#"        .arg("build")"#);
                }
                writeln!(s, r#"        .arg("--target-dir={}")"#, config.target_dir.display());
                if config.out_dir_has_triple {
                    writeln!(s, r#"        .arg("--target={}")"#, config.target_triple);
                }
                if !test.no_run {
                    write!(s, r#"        .arg("--manifest-path={}/{}/{}")"#, config.test_dir.display(), "master_skeptic", "Cargo.toml");
                } else {
                    write!(s, r#"        .arg("--manifest-path={}/{}/{}")"#, config.test_dir.display(), test.name, "Cargo.toml");
                }
                writeln!(s, r#"        .arg("-Zunstable-options")"#);
                //writeln!(s, r#"        .arg("-Zoffline")"#);
                if !test.no_run {
                    writeln!(s);
                    writeln!(s, r#"        .arg("--")"#);
                    writeln!(s, r#"        .arg("{}");"#, test.name);
                } else {
                    writeln!(s, r#";"#);
                }
                writeln!(s);

                writeln!(s, r#"    let res = cmd.status()"#);
                writeln!(s, r#"        .expect("cargo failed to run for test {}");"#, test.name);
                writeln!(s);

                writeln!(s, r#"    if !res.success() {{"#);
                if !test.no_run {
                    writeln!(s, r#"        panic!("cargo run {} failed")"#, test.name);
                } else {
                    writeln!(s, r#"        panic!("cargo build {} failed")"#, test.name);
                }
                writeln!(s, r#"    }}"#);

                writeln!(s, "}}"); // 'fn' closer

                if test.ignore { writeln!(s, "*/"); }

                writeln!(buf, "{}", s);
                writeln!(buf);
            }
        }

        fs::create_dir_all(&config.test_dir)?;

        write_if_contents_changed(&config.test_file, &buf)?;

        Ok(())
    }

    fn emit_test_projects(config: &Config, suite: &DocTestSuite) -> Result<(), Box<StdError + Send + Sync + 'static>> {
        for test_doc in &suite.doc_tests {
            for test in &test_doc.tests {
                emit_test_project(config, test_doc, test)?;
            }
        }

        Ok(())
    }

    fn emit_test_project(config: &Config, test_doc: &DocTest, test: &Test) -> Result<(), Box<StdError + Send + Sync + 'static>> {
        let test_name = &test.name;
        let test_src = build_test_src(&test_doc, &test);

        emit_project(&config.test_dir, test_name, &test_src,
                     &config.manifest_info, LibOrBin::Lib)
    }

    fn emit_project(test_dir: &Path, test_name: &str, test_src: &str,
                    manifest_info: &ManifestInfo, lib_bin: LibOrBin) -> Result<(), Box<StdError + Send + Sync + 'static>> {

        let mut test_dir = test_dir.to_owned();
        test_dir.push(test_name.to_string());

        let mut test_manifest = test_dir.clone();
        test_manifest.push("Cargo.toml");

        let mut test_src_file = test_dir.clone();
        test_src_file.push("test.rs");

        let manifest = build_manifest(manifest_info, &test_name, lib_bin);
        let manifest_str = toml::to_string_pretty(&manifest)?;

        fs::create_dir_all(&test_dir)?;

        write_if_contents_changed(&test_manifest, &manifest_str)?;
        write_if_contents_changed(&test_src_file, test_src)?;

        Ok(())
    }

    #[derive(Eq, PartialEq)]
    enum LibOrBin { Lib, Bin }

    fn build_manifest(info: &ManifestInfo, test_name: &str, lib_bin: LibOrBin) -> Value {
        let mut toml_map = BTreeMap::new();

        // insert sections inherited from the doc project
        {
            if let Some(ref deps) = info.deps {
                toml_map.insert("dependencies".to_string(), deps.to_owned());
            }
            if let Some(ref deps) = info.dev_deps {
                toml_map.insert("dev-dependencies".to_string(), deps.to_owned());
            }
            if let Some(ref deps) = info.build_deps {
                toml_map.insert("build-dependencies".to_string(), deps.to_owned());
            }
        }

        // insert 'lib' section
        if lib_bin == LibOrBin::Lib {
            let mut test_map = BTreeMap::new();

            test_map.insert("name".to_string(), Value::String(test_name.to_string()));
            test_map.insert("path".to_string(), Value::String("test.rs".to_string()));

            toml_map.insert("lib".to_string(), Value::Table(test_map));
        }

        // insert 'bin' section
        if lib_bin == LibOrBin::Bin {
            let mut test_map = BTreeMap::new();

            test_map.insert("name".to_string(), Value::String(test_name.to_string()));
            test_map.insert("path".to_string(), Value::String("test.rs".to_string()));

            toml_map.insert("bin".to_string(), Value::Array(vec![Value::Table(test_map)]));
        }

        // insert 'project' section
        {
            let mut proj_map = BTreeMap::new();

            proj_map.insert("name".to_string(), Value::String(test_name.to_string()));
            proj_map.insert("version".to_string(), Value::String("0.0.0".to_string()));
            proj_map.insert("authors".to_string(), Value::Array(vec![Value::String("rust-skeptic".to_string())]));

            toml_map.insert("project".to_string(), Value::Table(proj_map));
        }

        Value::Table(toml_map)
    }

    fn build_test_src(test_doc: &DocTest, test: &Test) -> String {
        let template = get_template(test_doc, test);
        let test_text = create_test_input(&test.text);
        let s = compose_template(&template, test_text);

        let mut s = format!("#![feature(termination_trait_lib)] // skeptic\n\n{}", s);

        writeln!(s);
        writeln!(s, r#"pub fn __skeptic_main() -> i32 {{"#);
        writeln!(s, r#"    use std::process::Termination;"#);
        writeln!(s, r#"    let r = main();"#);
        writeln!(s, r#"    let exit_code = r.report();"#);
        writeln!(s, r#"    if exit_code != 0 {{"#);
        writeln!(s, r#"        println!("{{:#?}}", r);"#);
        writeln!(s, r#"    }}"#);
        writeln!(s, r#"    exit_code"#);
        writeln!(s, r#"}}"#);

        s
    }

    fn get_template(test_doc: &DocTest, test: &Test) -> Option<String> {
        if let Some(ref t) = test.template {
            let template = test_doc.templates.get(t).expect(&format!(
                "template {} not found for {}",
                t,
                test_doc.path.display()
            ));
            Some(template.to_string())
        } else {
            test_doc.old_template.clone()
        }
    }

    // This is a hacky re-implementation of format! for runtime. It's not
    // going to be particularly reliable, and it only interprets "{ *}".
    // FIXME: This doesn't handle string literals that contain braces
    // TODO: Someday replace skeptic's templates with handlebars.
    fn compose_template(template: &Option<String>, test: String) -> String {

        fn is_odd(fuck_std_for_not_having_obvious_functions: usize) -> bool {
            let n = fuck_std_for_not_having_obvious_functions;
            !(n % 2 == 0)
        }
        
        if let Some(ref template) = template {
            enum State {
                Nothin,
                OpenBraceRun(Vec<usize>),
                Opener(usize),
                CloseBraceRun(Vec<usize>),
                CloseBraceRunWithOpener(usize, Vec<usize>),
            }

            let mut open_brace_runs = vec![];
            let mut close_brace_runs = vec![];
            let mut replacement = None;
            let mut state = State::Nothin;

            for (idx, ch) in template.chars().enumerate() {
                state = match state {
                    State::Nothin => {
                        match ch {
                            '{' => {
                                State::OpenBraceRun(vec![idx])
                            }
                            '}' => {
                                State::CloseBraceRun(vec![idx])
                            }
                            _ => {
                                State::Nothin
                            }
                        }
                    }
                    State::OpenBraceRun(mut open_braces) => {
                        match ch {
                            '{' => {
                                open_braces.push(idx);
                                State::OpenBraceRun(open_braces)
                            }
                            '}' => {
                                if is_odd(open_braces.len()) {
                                    let open_idx = open_braces.pop().unwrap();
                                    if !open_braces.is_empty() {
                                        open_brace_runs.push(open_braces);
                                    }
                                    State::CloseBraceRunWithOpener(open_idx, vec![idx])
                                } else {
                                    open_brace_runs.push(open_braces);
                                    State::CloseBraceRun(vec![idx])
                                }
                            }
                            _ => {
                                if ch.is_whitespace() {
                                    if is_odd(open_braces.len()) {
                                        let open_idx = open_braces.pop().unwrap();
                                        if !open_braces.is_empty() {
                                            open_brace_runs.push(open_braces);
                                        }
                                        State::Opener(open_idx)
                                    } else {
                                        open_brace_runs.push(open_braces);
                                        State::Nothin
                                    }
                                } else {
                                    open_brace_runs.push(open_braces);
                                    State::Nothin
                                }
                            }
                        }
                    }
                    State::Opener(open_idx) => {
                        match ch {
                            '{' => {
                                unreachable!();
                            }
                            '}' => {
                                State::CloseBraceRunWithOpener(open_idx, vec![idx])
                            }
                            _ => {
                                if ch.is_whitespace() {
                                    State::Opener(open_idx)
                                } else {
                                    State::Nothin
                                }
                            }
                        }
                    }
                    State::CloseBraceRun(mut close_braces) => {
                        match ch {
                            '{' => {
                                close_brace_runs.push(close_braces);
                                State::OpenBraceRun(vec![idx])
                            }
                            '}' => {
                                close_braces.push(idx);
                                State::CloseBraceRun(close_braces)
                            }
                            _ => {
                                close_brace_runs.push(close_braces);
                                State::Nothin
                            }
                        }
                    }
                    State::CloseBraceRunWithOpener(open_idx, mut close_braces) => {
                        match ch {
                            '{' => {
                                if is_odd(close_braces.len()) {
                                    if replacement.is_some() {
                                        panic!("multiple {{}} in skeptic template");
                                    }
                                    let mut close_braces = VecDeque::from(close_braces);
                                    let close_idx = close_braces.pop_front().unwrap();
                                    replacement = Some((open_idx, close_idx));
                                    if !close_braces.is_empty() {
                                        close_brace_runs.push(Vec::from(close_braces));
                                    }
                                    State::OpenBraceRun(vec![idx])
                                } else {
                                    close_brace_runs.push(close_braces);
                                    State::OpenBraceRun(vec![idx])
                                }
                            }
                            '}' => {
                                close_braces.push(idx);
                                State::CloseBraceRunWithOpener(open_idx, close_braces)
                            }
                            _ => {
                                if is_odd(close_braces.len()) {
                                    if replacement.is_some() {
                                        panic!("multiple {{}} in skeptic template");
                                    }
                                    let mut close_braces = VecDeque::from(close_braces);
                                    let close_idx = close_braces.pop_front().unwrap();
                                    replacement = Some((open_idx, close_idx));
                                    if !close_braces.is_empty() {
                                        close_brace_runs.push(Vec::from(close_braces));
                                    }
                                    State::Nothin
                                } else {
                                    close_brace_runs.push(close_braces);
                                    State::Nothin
                                }
                            }
                        }
                    }
                }
            } // for chars in template

            if !replacement.is_some() {
                panic!("no {{}} found in skeptic template");
            }

            let replacement = replacement.unwrap();
            let mut open_brace_runs = open_brace_runs;
            let mut close_brace_runs = close_brace_runs;

            for run in &mut open_brace_runs {
                if is_odd(run.len()) {
                    run.pop().unwrap();
                }
            }

            for run in &mut close_brace_runs {
                if is_odd(run.len()) {
                    run.pop().unwrap();
                }
            }

            let mut open_brace_runs = open_brace_runs.into_iter()
                .flat_map(|r| r.into_iter()).collect::<VecDeque<_>>();
            let mut close_brace_runs = close_brace_runs.into_iter()
                .flat_map(|r| r.into_iter()).collect::<VecDeque<_>>();

            let mut open_brace_pairs = VecDeque::new();
            let mut close_brace_pairs = VecDeque::new();

            while !open_brace_runs.is_empty() {
                let start = open_brace_runs.pop_front().unwrap();
                let end = open_brace_runs.pop_front().unwrap();
                open_brace_pairs.push_back((start, end))
            }
            while !close_brace_runs.is_empty() {
                let start = close_brace_runs.pop_front().unwrap();
                let end = close_brace_runs.pop_front().unwrap();
                close_brace_pairs.push_back((start, end))
            }

            let mut template_chars = template.chars().collect::<VecDeque<_>>();
            let template_chars_len = template_chars.len();

            let (rep_start, rep_end) = replacement;
            let mut open_brace_pairs = open_brace_pairs;
            let mut close_brace_pairs = close_brace_pairs;

            // Build the test src while replacing {{, }}, and {}
            let mut src = String::new();

            writeln!(src, "// rep_start, rep_end: {}, {}", rep_start, rep_end);
            writeln!(src, "// open_brace_pairs: {:?}", open_brace_pairs);
            writeln!(src, "// close_brace_pairs: {:?}", close_brace_pairs);

            let mut idx = 0;
            while idx != template_chars_len {
                let ch = template_chars.pop_front().unwrap();

                if idx == rep_start {
                    src.push_str(&test);
                    idx += 1;
                    while idx <= rep_end {
                        let ch = template_chars.pop_front().unwrap();
                        idx += 1;
                    }
                } else if open_brace_pairs.front().cloned().map(|(start, _)| start) == Some(idx) {
                    let (open_start, open_end) = open_brace_pairs.pop_front().unwrap();
                    assert!(open_start + 1 == open_end);
                    template_chars.pop_front().unwrap();
                    src.push('{');
                    idx += 2;
                } else if close_brace_pairs.front().cloned().map(|(start, _)| start) == Some(idx) {
                    let (close_start, close_end) = close_brace_pairs.pop_front().unwrap();
                    assert!(close_start + 1 == close_end);
                    template_chars.pop_front().unwrap();
                    src.push('}');
                    idx += 2;
                } else {
                    src.push(ch);
                    idx += 1;
                }
            }

            src
        } else {
            test
        }
    }

    /// Just like Rustdoc, ignore a "#" sign at the beginning of a line of code.
    /// These are commonly an indication to omit the line from user-facing
    /// documentation but include it for the purpose of playground links or skeptic
    /// testing.
    fn clean_omitted_line(line: &str) -> &str {
        let trimmed = line.trim_left();

        if trimmed.starts_with("# ") {
            &trimmed[2..]
        } else if trimmed.trim_right() == "#" {
            // line consists of single "#" which might not be followed by newline on windows
            &trimmed[1..]
        } else {
            line
        }
    }

    /// Creates the Rust code that this test will be operating on.
    fn create_test_input(lines: &[String]) -> String {
        lines
            .iter()
            .map(|s| clean_omitted_line(s).to_owned())
            .collect()
    }

    fn emit_supercrate_project(config: &Config, suite: &DocTestSuite) -> Result<(), Box<StdError + Send + Sync + 'static>> {
        let test_name = "master_skeptic";
        let test_src = build_supercrate_src(config, suite);
        let manifest_info = build_supercrate_manifest(config, suite);

        emit_project(&config.test_dir, &test_name, &test_src,
                     &manifest_info, LibOrBin::Bin)
    }

    fn build_supercrate_src(config: &Config, suite: &DocTestSuite) -> String {
        let mut s = String::new();

        let mut sb = String::new();
        for test_doc in &suite.doc_tests {
            for test in &test_doc.tests {
                if !(test.ignore || test.no_run) {
                    writeln!(sb, r#"    if test_name == "{}" {{"#, test.name);
                    writeln!(sb, r#"        exit_code = {}::__skeptic_main();"#, test.name);
                    writeln!(sb, r#"    }}"#);
                    writeln!(sb);
                }
            }
        }

        let switch_buf = sb;

        writeln!(s, r#"
fn main() {{

    let test_name = std::env::args().skip(1).next().expect("arg 1 is test name");

    let mut exit_code = 0;

{}

    std::process::exit(exit_code);
}}"#,
                 switch_buf,
                 
        );

        s
    }

    fn build_supercrate_manifest(config: &Config, suite: &DocTestSuite) -> ManifestInfo {

        let mut deps = BTreeMap::new();
        
        for test_doc in &suite.doc_tests {
            for test in &test_doc.tests {
                if !test.ignore && !test.no_run {
                    let mut props = BTreeMap::new();
                    let path = format!("../{}", test.name.clone());
                    props.insert("path".to_string(), Value::String(path));

                    deps.insert(test.name.clone(), Value::Table(props));
                }
            }
        }
        
        ManifestInfo {
            deps: Some(Value::Table(deps)),
            dev_deps: None,
            build_deps: None,
        }
    }

    fn write_if_contents_changed(name: &Path, contents: &str) -> Result<(), IoError> {
        use std::io::Write;

        let out_dir = name.parent().expect("test path name should contain a directory and file");
        fs::create_dir_all(out_dir)?;

        // Can't open in write mode now as that would modify the last changed timestamp of the file
        match File::open(name) {
            Ok(mut file) => {
                let mut current_contents = String::new();
                file.read_to_string(&mut current_contents)?;
                if current_contents == contents {
                    // No change avoid writing to avoid updating the timestamp of the file
                    return Ok(());
                }
            }
            Err(ref err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }
        let mut file = File::create(name)?;
        file.write_all(contents.as_bytes())?;
        Ok(())
    }

}
