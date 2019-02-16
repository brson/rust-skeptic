#![allow(warnings)] // todo

use std::collections::{BTreeMap, VecDeque};
use std::error::Error as StdError;
use std::fmt::Write;
use std::fs::{self, File};
use std::io::{self, Read, Error as IoError};
use std::path::{PathBuf, Path};
use super::{Config, DocTestSuite, DocTest, Test, Manifest};
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
    writeln!(buf, r#"
static CARGO: &str = "{}";
static RUSTC: &str = "{}";
static TARGET_DIR: &str = "{}";
static TARGET_EXE_DIR: &str = "{}";
static MANIFEST: &str = "{}/master_skeptic/Cargo.toml";
static TARGET_TRIPLE: &str = "{}";
static OUT_DIR_HAS_TRIPLE: bool = {};"#,
             config.cargo,
             config.rustc,
             config.target_dir.display(),
             config.target_exe_dir.display(),
             config.test_dir.display(),
             config.target_triple,
             config.out_dir_has_triple,
    );
    writeln!(buf);
    writeln!(buf);

    for test_doc in &suite.doc_tests {
        for test in &test_doc.tests {
            let mut s = String::new();

            writeln!(s, "// file {}; line {}", test_doc.short_path.display(), test.line);
            if test.ignore { writeln!(s, "/*"); }
            if test.ignore { writeln!(s, "// skeptic-ignored test"); }
            if test.no_run { writeln!(s, "// skeptic-no_run test"); }
            if test.should_panic { writeln!(s, "#[should_panic]"); }

            writeln!(s,
r#"#[test]
fn {test_name}() {{
    static TEST_NAME: &str = "{test_name}";
    static NO_RUN: bool = {no_run};
    static SHORT_PATH: &str = "{short_path}";
    static LINE_NUMBER: usize = {line_number};
    run_test(TEST_NAME, NO_RUN, SHORT_PATH, LINE_NUMBER);
}}"#,
                     test_name = test.name,
                     no_run = test.no_run,
                     short_path = test_doc.short_path.display(),
                     line_number = test.line,
            );

            if test.ignore { writeln!(s, "*/"); }

            writeln!(buf, "{}", s);
            writeln!(buf);
        }
    }

    writeln!(buf, r#"
fn run_test(test_name: &str, no_run: bool, short_path: &str, line_no: usize) {{
    // todo: --release, --nocapture

    wait_for_resolve_and_fetch();

    if !no_run {{
        wait_for_build_master_skeptic();
        run_master_skeptic_test(test_name, short_path, line_no);
    }} else {{
        run_no_run_test(test_name, short_path, line_no);
    }}
}}

fn wait_for_resolve_and_fetch() {{

    static INIT: std::sync::Once = std::sync::ONCE_INIT;

    INIT.call_once(|| {{

        let mut cmd = Command::new(CARGO);
        cmd
            .arg("generate-lockfile")
            .arg(&format!("--manifest-path={{}}", MANIFEST))
            .env("RUSTC_BOOTSTRAP", "1")
            .arg("-Zunstable-options")
            .arg("-Zno-index-update")
            .arg("-Zoffline");

        let res = cmd.status()
            .expect("skeptic failed to run cargo to generate master_skeptic lockfile");

        if !res.success() {{
            panic!("failed to initialize master_skeptic");
        }}

        let mut cmd = Command::new(CARGO);
        cmd
            .arg("fetch")
            .arg(&format!("--manifest-path={{}}", MANIFEST))
            .env("RUSTC_BOOTSTRAP", "1")
            .arg("-Zunstable-options")
            .arg("-Zoffline");

        let res = cmd.status()
            .expect("skeptic failed to run cargo to fetch master_skeptic deps");

        if !res.success() {{
            panic!("failed to initialize master_skeptic");
        }}

    }});
}}

fn wait_for_build_master_skeptic() {{

    static INIT: std::sync::Once = std::sync::ONCE_INIT;

    INIT.call_once(|| {{

        let mut cmd = Command::new(CARGO);
        cmd
            .env("RUSTC", RUSTC)
            // ... shhhhhh ... this gives access to the Termination trait
            .env("RUSTC_BOOTSTRAP", "1")
            .arg("build")
            .arg(&format!("--manifest-path={{}}", MANIFEST))
            .arg(&format!("--target-dir={{}}", TARGET_DIR))
            .env("RUSTC_BOOTSTRAP", "1")
            .arg("-Zunstable-options")
            .arg("-Zoffline");

        if OUT_DIR_HAS_TRIPLE {{
            cmd.arg(&format!("--target={{}}", TARGET_TRIPLE));
        }}

        let res = cmd.status()
            .expect(&format!("cargo failed to run for master skeptic"));

        if !res.success() {{
            panic!("failed to initialize master_skeptic");
        }}

    }});
}}

fn run_master_skeptic_test(test_name: &str, short_path: &str, line_no: usize) {{

    let exe = format!("{{}}/master_skeptic", TARGET_EXE_DIR);

    let res = Command::new(exe)
        .env("SKEPTIC_TEST_NAME", test_name)
        .status();
    let res = res.expect("failed to execute bin for master_skeptic test");

    if !res.success() {{
        panic!("test {{}} - line {{}} failed, test {{}}",
               short_path, line_no, test_name);
    }}
}}

fn run_no_run_test(test_name: &str, short_path: &str, line_no: usize) {{

    let mut cmd = Command::new(CARGO);
    cmd
        .env("RUSTC", RUSTC)
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("build")
        .arg(&format!("--manifest-path={{}}", MANIFEST))
        .arg(&format!("--target-dir={{}}", TARGET_DIR))
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("-Zunstable-options")
        .arg("-Zoffline");

    if OUT_DIR_HAS_TRIPLE {{
        cmd.arg(&format!("--target={{}}", TARGET_TRIPLE));
    }}

    cmd
        .arg("--frozen")
        .arg("--locked")
        .arg("-p")
        .arg(&format!("{{}}", test_name));

    let res = cmd.status()
        .expect(&format!("cargo failed to run for test {{}}", test_name));

    if !res.success() {{
        panic!("cargo build for {{}} - line {{}} failed, test {{}}",
               short_path, line_no, test_name);
    }}
}}

"#);

    fs::create_dir_all(&config.test_dir)?;

    write_if_contents_changed(&config.test_file, &buf)?;

    Ok(())
}

fn emit_test_projects(config: &Config, suite: &DocTestSuite) -> Result<(), Box<StdError + Send + Sync + 'static>> {
    for test_doc in &suite.doc_tests {
        for test in &test_doc.tests {
            emit_test_project(config, test_doc, test, &suite.manifest)?;
        }
    }

    Ok(())
}

fn emit_test_project(config: &Config, test_doc: &DocTest, test: &Test,
                     client_manifest: &Manifest) -> Result<(), Box<StdError + Send + Sync + 'static>> {
    let test_name = &test.name;
    let test_src = build_test_src(&test_doc, &test);

    emit_project(&config.test_dir, test_name, &test_src,
                 client_manifest, LibOrBin::Lib)
}

fn emit_project(test_dir: &Path, test_name: &str, test_src: &str,
                template_manifest: &Manifest, lib_bin: LibOrBin) -> Result<(), Box<StdError + Send + Sync + 'static>> {

    let mut test_dir = test_dir.to_owned();
    if test_name != "master_skeptic" {
        test_dir.push("master_skeptic");
    }
    test_dir.push(test_name.to_string());

    let mut test_manifest = test_dir.clone();
    test_manifest.push("Cargo.toml");

    let mut test_src_file = test_dir.clone();
    test_src_file.push("test.rs");

    let manifest = build_manifest(template_manifest, &test_name, lib_bin);
    let manifest_str = toml::to_string_pretty(&manifest)?;

    fs::create_dir_all(&test_dir)?;

    write_if_contents_changed(&test_manifest, &manifest_str)?;
    write_if_contents_changed(&test_src_file, test_src)?;

    Ok(())
}

fn build_test_src(test_doc: &DocTest, test: &Test) -> String {
    let template = get_template(test_doc, test);
    let test_text = create_test_input(&test.text);
    let test_src = compose_template(&template, test_text);

    let mut s = String::new();

    writeln!(s, "// file {}, line {}", test_doc.short_path.display(), test.line);
    writeln!(s, "// test {}", test.name);
    writeln!(s, "#![feature(termination_trait_lib)] // skeptic");
    writeln!(s);
    writeln!(s, "{}", test_src);
    writeln!(s);
    writeln!(s, r#"pub fn __skeptic_main() -> i32 {{"#);
    writeln!(s, r#"    use std::process::Termination;"#);
    writeln!(s, r#"    main().report()"#);
    writeln!(s, r#"}}"#);

    s
}

#[derive(Eq, PartialEq, Copy, Clone)]
enum LibOrBin { Lib, Bin }

fn build_manifest(template_manifest: &Manifest, test_name: &str, lib_bin: LibOrBin) -> Value {
    let mut toml_map = BTreeMap::new();

    // insert sections inherited from the doc project
    {
        if let Value::Table(sections) = &template_manifest.0 {
            for (sec_key, sec_value) in sections {
                match sec_key.as_str() {
                    "features" => {
                        toml_map.insert(sec_key.clone(), sec_value.clone());
                    }
                    "workspace" => {
                        toml_map.insert(sec_key.clone(), sec_value.clone());
                    }
                    "dependencies" => {
                        toml_map.insert(sec_key.clone(), sanitize_deps(sec_value.clone(), lib_bin));
                    }
                    "dev-dependencies" => {
                        toml_map.insert(sec_key.clone(), sanitize_deps(sec_value.clone(), lib_bin));
                    }
                    "build-dependencies" => {
                        toml_map.insert(sec_key.clone(), sanitize_deps(sec_value.clone(), lib_bin));
                    }
                    "target" => {

                        match sec_value.clone() {
                            Value::Table(targets) => {
                                let mut new_targets = BTreeMap::new();
                                for (target, sections) in targets {
                                    let mut new_sections = BTreeMap::new();
                                    match sections {
                                        Value::Table(sections) => {
                                            for (section_name, props) in sections {
                                                match section_name.as_str() {
                                                    "dependencies" => {
                                                        new_sections.insert(section_name, sanitize_deps(props, lib_bin));
                                                    }
                                                    _ => { }
                                                }
                                            }
                                        }
                                        _ => { panic!("unexpected blah blah blah"); }
                                    }
                                    new_targets.insert(target, Value::Table(new_sections));
                                } // for _ in targets

                                toml_map.insert(sec_key.clone(), Value::Table(new_targets));
                            }
                            _ => {
                                panic!("unexpected target type in manifest");
                            }
                        }
                    }
                    _ => { }
                }
            }
        } else {
            panic!("unexpected toml type in manifest {:?}", template_manifest);
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

fn sanitize_deps(toml: Value, lib_bin: LibOrBin) -> Value {
    // Hack: we don't want to run this on the "master_skeptic" project,
    // and we know that it is the only bin project.
    if lib_bin == LibOrBin::Bin {
        return toml;
    }
    
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
                                // of the test manifest, "tests/skeptic/master_skeptic/$test_name/"
                                // FIXME: This only works  if the path isn't absolute.
                                let mut prop_value = format!("../../../../{}", prop_value);
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
    let template_manifest = build_supercrate_manifest_template(config, suite);

    emit_project(&config.test_dir, &test_name, &test_src,
                 &template_manifest, LibOrBin::Bin)
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

    let test_name = std::env::var("SKEPTIC_TEST_NAME")
        .expect("SKEPTIC_TEST_NAME not set");

    let mut exit_code = 0;

{}

    std::process::exit(exit_code);
}}
"#,
             switch_buf,
    );

    s
}

fn build_supercrate_manifest_template(config: &Config, suite: &DocTestSuite) -> Manifest {

    let mut sections = BTreeMap::new();

    {
        let mut deps = BTreeMap::new();
        let mut workspace_members = vec![];
        //let mut features = vec![];
        
        for test_doc in &suite.doc_tests {
            for test in &test_doc.tests {
                if !test.ignore {
                    let mut props = BTreeMap::new();
                    let path = format!("{}", test.name.clone());
                    props.insert("path".to_string(), Value::String(path));
                    if test.no_run {
                        props.insert("optional".to_string(), Value::Boolean(true));
                    }
                    deps.insert(test.name.clone(), Value::Table(props));
                    workspace_members.push(Value::String(format!("{}", test.name)));
                }
            }
        }
        
        let mut ws_props = BTreeMap::new();
        ws_props.insert("members".to_string(), Value::Array(workspace_members));
        sections.insert("workspace".to_string(), Value::Table(ws_props));

        sections.insert("dependencies".to_string(), Value::Table(deps));
    }

    Manifest(Value::Table(sections))
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

