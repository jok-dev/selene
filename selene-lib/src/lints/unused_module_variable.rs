use crate::ast_util::scopes::{AssignedValue, Reference};

use super::{
    unused_variable::{analyze_reference, AnalyzedReference},
    *,
};

use full_moon::node::Node;
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct UnusedModuleVariableConfig {
    #[serde(alias = "ignore_module_fields")]
    ignore_fields: Vec<String>,
}

impl Default for UnusedModuleVariableConfig {
    fn default() -> Self {
        Self {
            ignore_fields: vec!["__index".to_owned()],
        }
    }
}

pub struct UnusedModuleVariableLint {
    ignore_fields: HashSet<String>,
}

#[derive(Default)]
struct StaticFieldUsage {
    has_read: bool,
    has_write: bool,
    first_write_label: Option<Label>,
}

fn get_static_field_write(reference: &Reference) -> Option<(String, Label)> {
    let indexing = reference.indexing.as_ref()?;
    if indexing.len() != 1 {
        return None;
    }

    let static_name = indexing[0].static_name.as_ref()?;
    let (start, end) = static_name.range()?;

    Some((
        static_name.token().to_string(),
        Label::new((start.bytes() as u32, end.bytes() as u32)),
    ))
}

fn collect_lua_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            collect_lua_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            files.push(path);
        }
    }
}

fn find_external_module_field_reads(
    context: &Context,
    field_names: impl Iterator<Item = String>,
) -> HashSet<String> {
    let Some(root_path) = &context.root_path else {
        return HashSet::new();
    };

    let Some(current_file) = &context.current_file else {
        return HashSet::new();
    };

    let current_file_path = if current_file.is_absolute() {
        current_file.clone()
    } else {
        root_path.join(current_file)
    };

    let Some(module_name) = current_file_path.file_stem().and_then(|stem| stem.to_str()) else {
        return HashSet::new();
    };

    let require_regex = match Regex::new(&format!(
        r#"local\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*require\(\s*["']{}["']\s*\)"#,
        regex::escape(module_name)
    )) {
        Ok(regex) => regex,
        Err(_) => return HashSet::new(),
    };

    let mut lua_files = Vec::new();
    collect_lua_files(root_path, &mut lua_files);

    let mut used_fields = HashSet::new();
    let field_names = field_names.collect::<Vec<_>>();

    for path in lua_files {
        if path == current_file_path {
            continue;
        }

        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };

        for captures in require_regex.captures_iter(&source) {
            let Some(alias) = captures.get(1).map(|capture| capture.as_str()) else {
                continue;
            };

            for field_name in &field_names {
                let field_regex = match Regex::new(&format!(
                    r"\b{}\s*\.\s*{}\b",
                    regex::escape(alias),
                    regex::escape(field_name)
                )) {
                    Ok(regex) => regex,
                    Err(_) => continue,
                };

                if field_regex.is_match(&source) {
                    used_fields.insert(field_name.clone());
                }
            }
        }
    }

    used_fields
}

fn normalize_field_name(field_name: &str) -> String {
    field_name.trim_start_matches('.').to_owned()
}

fn default_ignored_fields() -> HashSet<String> {
    ["__index".to_owned()].into_iter().collect()
}

impl Lint for UnusedModuleVariableLint {
    type Config = UnusedModuleVariableConfig;
    type Error = std::convert::Infallible;

    const SEVERITY: Severity = Severity::Warning;
    const LINT_TYPE: LintType = LintType::Style;

    fn new(config: Self::Config) -> Result<Self, Self::Error> {
        let mut ignore_fields = default_ignored_fields();
        ignore_fields.extend(
            config
                .ignore_fields
                .into_iter()
                .map(|field_name| normalize_field_name(&field_name)),
        );

        Ok(Self { ignore_fields })
    }

    fn pass(
        &self,
        _: &full_moon::ast::Ast,
        context: &Context,
        ast_context: &AstContext,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (_, variable) in ast_context.scope_manager.variables.iter() {
            if variable.is_parameter
                || !matches!(variable.value, Some(AssignedValue::StaticTable { .. }))
            {
                continue;
            }

            let variable_is_read = variable.references.iter().copied().any(|id| {
                analyze_reference(
                    variable,
                    &ast_context.scope_manager.references[id],
                    context,
                    ast_context,
                ) == AnalyzedReference::Read
            });

            if !variable_is_read {
                continue;
            }

            let mut static_field_usage = BTreeMap::<String, StaticFieldUsage>::new();

            for reference in variable
                .references
                .iter()
                .copied()
                .map(|id| &ast_context.scope_manager.references[id])
            {
                let Some((field_name, write_label)) = get_static_field_write(reference) else {
                    continue;
                };

                let usage = static_field_usage.entry(field_name).or_default();

                if reference.write.is_some() {
                    usage.has_write = true;
                    if usage.first_write_label.is_none() {
                        usage.first_write_label = Some(write_label);
                    }
                }

                if reference.write.is_none() && reference.read {
                    usage.has_read = true;
                }
            }

            let external_reads =
                find_external_module_field_reads(context, static_field_usage.keys().cloned());

            for (field_name, usage) in static_field_usage {
                if self.ignore_fields.contains(&field_name) {
                    continue;
                }

                if usage.has_write && !usage.has_read && !external_reads.contains(&field_name) {
                    diagnostics.push(Diagnostic::new(
                        "unused_module_variable",
                        format!(
                            "{}.{} is assigned a value, but never used",
                            variable.name, field_name
                        ),
                        usage
                            .first_write_label
                            .expect("static table field write should have a label"),
                    ));
                }
            }
        }

        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::{super::test_util::test_lint, *};
    use crate::{
        test_util::{get_standard_library, PrettyString},
        Checker, CheckerConfig, Severity, StandardLibrary,
    };
    use codespan_reporting::{
        diagnostic::Severity as CodespanSeverity, term::Config as CodespanConfig,
    };
    use std::{fs, path::Path};

    #[test]
    fn test_module_fields() {
        let path_base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("lints")
            .join("unused_module_variable")
            .join("module_fields");

        let checker = Checker::<serde_json::Value>::new(
            CheckerConfig::default(),
            get_standard_library(&path_base).unwrap_or_else(|| {
                StandardLibrary::from_name("lua51").expect("no lua51 standard library")
            }),
            Some(
                path_base
                    .parent()
                    .expect("module_fields fixture should have a parent directory")
                    .to_path_buf(),
            ),
        )
        .expect("couldn't create checker");

        let lua_path = path_base.with_extension("lua");
        let lua_source = fs::read_to_string(&lua_path).expect("Cannot find lua file");
        let ast = full_moon::parse(&lua_source).expect("Cannot parse lua file");
        let mut diagnostics = checker.test_on(&ast, Some(lua_path.clone()));

        let mut files = codespan::Files::new();
        let source_id = files.add("module_fields.lua".to_owned(), lua_source);

        diagnostics.sort_by_key(|diagnostic| diagnostic.diagnostic.primary_label.range);

        let mut output = termcolor::NoColor::new(Vec::new());

        for diagnostic in diagnostics.into_iter().filter_map(|diagnostic| {
            Some(diagnostic.diagnostic.into_codespan_diagnostic(
                source_id,
                match diagnostic.severity {
                    Severity::Allow => return None,
                    Severity::Error | Severity::Warning => CodespanSeverity::Error,
                },
            ))
        }) {
            codespan_reporting::term::emit(
                &mut output,
                &CodespanConfig::default(),
                &files,
                &diagnostic,
            )
            .expect("couldn't emit to codespan");
        }

        let stderr = std::str::from_utf8(output.get_ref()).expect("output not utf-8");
        let expected = fs::read_to_string(path_base.with_extension("stderr"))
            .expect("Cannot find stderr file")
            .replace("\r\n", "\n");

        pretty_assertions::assert_eq!(PrettyString(&expected), PrettyString(stderr));
    }

    #[test]
    fn test_module_fields_custom_ignores() {
        test_lint(
            UnusedModuleVariableLint::new(UnusedModuleVariableConfig {
                ignore_fields: vec![".Attributes".to_owned(), ".Tag".to_owned()],
            })
            .unwrap(),
            "unused_module_variable",
            "module_fields_custom_ignores",
        );
    }
}
