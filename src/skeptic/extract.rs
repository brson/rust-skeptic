#![allow(warnings)] // todo

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fs::File;
use std::io::{Read, Error as IoError};
use std::mem;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use super::{Config, Test, DocTest, DocTestSuite, Manifest};
use cmark::{Parser, Event, Tag};
use toml::Value;

pub (in super) fn extract_tests(config: &Config) -> Result<DocTestSuite, IoError> {
    let mut doc_tests = Vec::new();
    for doc in &config.docs {
        let path = &mut config.root_dir.clone();
        path.push(doc);
        let new_tests = extract_tests_from_file(path)?;
        doc_tests.push(new_tests);
    }

    let manifest = load_manifest(&config.root_dir)
        .expect("unable to parse manifest for skeptic test generation");
    
    Ok(DocTestSuite { doc_tests, manifest })
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

fn load_manifest(manifest_dir: &Path) -> Result<Manifest, Box<StdError + Sync + Send + 'static>> {

    use std::fs::File;
    use std::io::Read;
    use toml::Value;

    let mut manifest = manifest_dir.to_owned();
    manifest.push("Cargo.toml");

    let mut manifest_buf = String::new();

    File::open(manifest)?.read_to_string(&mut manifest_buf)?;

    let manifest = manifest_buf.parse::<Value>()?;

    Ok(Manifest(manifest))
}
