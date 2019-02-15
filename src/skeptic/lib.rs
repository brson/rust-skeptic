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

mod extract;
mod emit;

