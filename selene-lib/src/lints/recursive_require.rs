use super::*;
use crate::ast_util::scopes::ScopeManager;

use full_moon::{
    ast::{self, Ast, Expression},
    node::Node,
    tokenizer::{Symbol, TokenType},
    visitors::Visitor,
};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct RecursiveRequireLint;

impl Lint for RecursiveRequireLint {
    type Config = ();
    type Error = Infallible;

    const SEVERITY: Severity = Severity::Warning;
    const LINT_TYPE: LintType = LintType::Correctness;

    fn new(_: Self::Config) -> Result<Self, Self::Error> {
        Ok(Self)
    }

    fn pass(&self, ast: &Ast, context: &Context, ast_context: &AstContext) -> Vec<Diagnostic> {
        let Some(root_path) = context.root_path.as_deref() else {
            return Vec::new();
        };

        let Some(current_file) = context.current_file.as_deref() else {
            return Vec::new();
        };

        let root_path = canonicalize_path(root_path).unwrap_or_else(|| root_path.to_path_buf());
        let current_file_path = canonicalize_path(&absolutize_path(&root_path, current_file))
            .unwrap_or_else(|| absolutize_path(&root_path, current_file));

        let require_sites = collect_require_sites(
            ast,
            &root_path,
            &current_file_path,
            &ast_context.scope_manager,
        );

        let mut dependency_graph = DependencyGraph::new(&root_path);
        let mut diagnostics = Vec::new();

        for require_site in require_sites {
            let cycle = if require_site.target_path == current_file_path {
                Some(vec![current_file_path.clone()])
            } else {
                dependency_graph.path_to_target(
                    &require_site.target_path,
                    &current_file_path,
                    &mut HashSet::new(),
                )
            };

            let Some(cycle) = cycle else {
                continue;
            };

            diagnostics.push(Diagnostic::new_complete(
                "recursive_require",
                "this require participates in a recursive dependency".to_owned(),
                require_site.label,
                vec![format!(
                    "dependency chain: {}",
                    format_cycle_chain(&root_path, &current_file_path, &cycle)
                )],
                Vec::new(),
            ));
        }

        diagnostics
    }
}

struct RequireSite {
    label: Label,
    target_path: PathBuf,
}

struct DependencyGraph<'a> {
    root_path: &'a Path,
    dependencies: HashMap<PathBuf, Vec<PathBuf>>,
}

impl<'a> DependencyGraph<'a> {
    fn new(root_path: &'a Path) -> Self {
        Self {
            root_path,
            dependencies: HashMap::new(),
        }
    }

    fn path_to_target(
        &mut self,
        source: &Path,
        target: &Path,
        visiting: &mut HashSet<PathBuf>,
    ) -> Option<Vec<PathBuf>> {
        if source == target {
            return Some(vec![target.to_path_buf()]);
        }

        let source = source.to_path_buf();
        if !visiting.insert(source.clone()) {
            return None;
        }

        let dependencies = self.dependencies_for(&source).clone();
        for dependency in dependencies {
            if let Some(mut path) = self.path_to_target(&dependency, target, visiting) {
                let mut cycle = vec![source.clone()];
                cycle.append(&mut path);
                visiting.remove(&source);
                return Some(cycle);
            }
        }

        visiting.remove(&source);
        None
    }

    fn dependencies_for(&mut self, file: &Path) -> &Vec<PathBuf> {
        self.dependencies
            .entry(file.to_path_buf())
            .or_insert_with(|| parse_file_dependencies(self.root_path, file))
    }
}

fn parse_file_dependencies(root_path: &Path, file: &Path) -> Vec<PathBuf> {
    let Ok(source) = fs::read_to_string(file) else {
        return Vec::new();
    };

    let Ok(ast) = full_moon::parse(&source) else {
        return Vec::new();
    };

    let ast_context = AstContext::from_ast(&ast);

    collect_require_sites(&ast, root_path, file, &ast_context.scope_manager)
        .into_iter()
        .map(|require_site| require_site.target_path)
        .collect()
}

fn collect_require_sites(
    ast: &Ast,
    root_path: &Path,
    current_file: &Path,
    scope_manager: &ScopeManager,
) -> Vec<RequireSite> {
    let mut visitor = RequireVisitor {
        current_file,
        root_path,
        scope_manager,
        function_depth: 0,
        ignored_require_starts: HashSet::new(),
        local_vars: HashMap::new(),
        require_sites: Vec::new(),
    };

    visitor.visit_ast(ast);
    visitor.require_sites
}

struct RequireVisitor<'a> {
    current_file: &'a Path,
    root_path: &'a Path,
    scope_manager: &'a ScopeManager,
    function_depth: usize,
    ignored_require_starts: HashSet<usize>,
    local_vars: HashMap<String, String>,
    require_sites: Vec<RequireSite>,
}

impl RequireVisitor<'_> {
    fn require_argument<'a>(&self, call: &'a ast::FunctionCall) -> Option<&'a Expression> {
        if let Some(reference) = self
            .scope_manager
            .reference_at_byte(call.start_position()?.bytes())
        {
            if reference.resolved.is_some() {
                return None;
            }
        }

        let ast::Prefix::Name(name) = call.prefix() else {
            return None;
        };

        if name.to_string() != "require" {
            return None;
        }

        let Some(ast::Suffix::Call(ast::Call::AnonymousCall(ast::FunctionArgs::Parentheses {
            arguments,
            ..
        }))) = call.suffixes().next()
        else {
            return None;
        };

        if arguments.len() != 1 {
            return None;
        }

        arguments.iter().next()
    }

    fn expression_is_false(&self, expression: &Expression) -> bool {
        match expression {
            Expression::Parentheses { expression, .. } => self.expression_is_false(expression),
            Expression::Symbol(symbol) => matches!(
                symbol.token().token_type(),
                TokenType::Symbol {
                    symbol: Symbol::False
                }
            ),
            _ => false,
        }
    }

    fn disabled_require_start(&self, expression: &Expression) -> Option<usize> {
        match expression {
            Expression::Parentheses { expression, .. } => self.disabled_require_start(expression),
            Expression::FunctionCall(call) => {
                self.require_argument(call)?;
                Some(call.start_position()?.bytes())
            }
            _ => None,
        }
    }

    fn resolve_require_expression(&self, expression: &Expression) -> Option<PathBuf> {
        match expression {
            Expression::Parentheses { expression, .. } => {
                self.resolve_require_expression(expression)
            }

            Expression::String(token) => match token.token().token_type() {
                TokenType::StringLiteral { literal, .. } => {
                    resolve_string_require(self.current_file, self.root_path, literal)
                }
                _ => None,
            },

            _ => resolve_symbolic_require(
                self.current_file,
                self.root_path,
                &self.local_vars,
                expression.to_string(),
            ),
        }
    }
}

impl Visitor for RequireVisitor<'_> {
    fn visit_expression(&mut self, expression: &ast::Expression) {
        let ast::Expression::BinaryOperator { lhs, binop, rhs } = expression else {
            return;
        };

        if matches!(binop, ast::BinOp::And(_)) && self.expression_is_false(lhs) {
            if let Some(start) = self.disabled_require_start(rhs) {
                self.ignored_require_starts.insert(start);
            }
        }
    }

    fn visit_function_body(&mut self, _: &ast::FunctionBody) {
        self.function_depth += 1;
    }

    fn visit_function_body_end(&mut self, _: &ast::FunctionBody) {
        self.function_depth = self.function_depth.saturating_sub(1);
    }

    fn visit_local_assignment(&mut self, assignment: &ast::LocalAssignment) {
        if self.function_depth > 0 {
            return;
        }

        if let Some((name, expression)) = assignment
            .names()
            .iter()
            .next()
            .zip(assignment.expressions().iter().next())
        {
            self.local_vars.insert(
                name.to_string().trim().to_owned(),
                expression.to_string().trim().to_owned(),
            );
        }
    }

    fn visit_function_call(&mut self, call: &ast::FunctionCall) {
        if self.function_depth > 0 {
            return;
        }

        let Some(start) = call.start_position().map(|position| position.bytes()) else {
            return;
        };

        if self.ignored_require_starts.contains(&start) {
            return;
        }

        let Some(argument) = self.require_argument(call) else {
            return;
        };

        let Some(target_path) = self.resolve_require_expression(argument) else {
            return;
        };

        self.require_sites.push(RequireSite {
            label: Label::from_node(argument, None),
            target_path,
        });
    }
}

fn resolve_string_require(current_file: &Path, root_path: &Path, literal: &str) -> Option<PathBuf> {
    let current_directory = current_file.parent()?;

    for base_path in [
        path_from_module_name(current_directory, literal),
        path_from_module_name(root_path, literal),
    ] {
        if let Some(resolved_path) = resolve_file_candidate(&base_path) {
            return Some(resolved_path);
        }
    }

    None
}

fn resolve_symbolic_require(
    current_file: &Path,
    root_path: &Path,
    local_vars: &HashMap<String, String>,
    expression: String,
) -> Option<PathBuf> {
    let mut expression = strip_whitespace(&expression);
    expression = expand_local_aliases(local_vars, expression);
    expression = replace_wait_for_child_calls(&expression);

    if let Some(stripped) = expression.strip_prefix("game.") {
        expression = stripped.to_owned();
    }

    if expression == "script" {
        return Some(current_file.to_path_buf());
    }

    let segments = expression
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if segments.is_empty() {
        return None;
    }

    if segments[0] == "script" {
        return resolve_script_path(current_file, &segments[1..]);
    }

    resolve_root_path(root_path, &segments)
}

fn resolve_script_path(current_file: &Path, segments: &[&str]) -> Option<PathBuf> {
    let current_directory = current_file.parent()?;
    let mut base_path = current_directory.to_path_buf();
    let mut index = 0;

    while segments.get(index) == Some(&"Parent") {
        if index > 0 {
            base_path = base_path.parent()?.to_path_buf();
        }
        index += 1;
    }

    if index == segments.len() {
        return resolve_file_candidate(&base_path);
    }

    resolve_file_candidate(&join_segments(&base_path, &segments[index..]))
}

fn resolve_root_path(root_path: &Path, segments: &[&str]) -> Option<PathBuf> {
    resolve_file_candidate(&join_segments(root_path, segments))
}

fn join_segments(base_path: &Path, segments: &[&str]) -> PathBuf {
    let mut path = base_path.to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    path
}

fn path_from_module_name(base_path: &Path, module_name: &str) -> PathBuf {
    let mut path = base_path.to_path_buf();
    for segment in module_name.split(['.', '/', '\\']) {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    path
}

fn resolve_file_candidate(candidate: &Path) -> Option<PathBuf> {
    if candidate
        .extension()
        .and_then(|extension| extension.to_str())
        == Some("lua")
        && candidate.is_file()
    {
        return canonicalize_path(candidate);
    }

    let lua_file = candidate.with_extension("lua");
    if lua_file.is_file() {
        return canonicalize_path(&lua_file);
    }

    let init_file = candidate.join("init.lua");
    if init_file.is_file() {
        return canonicalize_path(&init_file);
    }

    None
}

fn canonicalize_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn absolutize_path(root_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root_path.join(path)
    }
}

fn strip_whitespace(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn expand_local_aliases(local_vars: &HashMap<String, String>, mut expression: String) -> String {
    for _ in 0..8 {
        let first_part = expression
            .split(['.', ':', '('])
            .next()
            .unwrap_or_default()
            .to_owned();

        let Some(replacement) = local_vars.get(&first_part) else {
            break;
        };

        expression = expression.replacen(&first_part, &strip_whitespace(replacement), 1);
    }

    expression
}

fn replace_wait_for_child_calls(input: &str) -> String {
    let mut processed = input.to_owned();
    let mut search_position = 0;

    while let Some(position) = processed[search_position..].find(":WaitForChild(") {
        let actual_position = search_position + position;

        let Some(close_position) = processed[actual_position..].find(')') else {
            break;
        };

        let close_position = actual_position + close_position;
        let inner_content = &processed[actual_position + ":WaitForChild(".len()..close_position];
        let module_name = inner_content
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_start_matches('\'')
            .trim_end_matches('\'');

        let replacement = format!(".{module_name}");
        processed = format!(
            "{}{}{}",
            &processed[..actual_position],
            replacement,
            &processed[close_position + 1..]
        );

        search_position = actual_position + replacement.len();
    }

    processed
}

fn format_cycle_chain(root_path: &Path, current_file: &Path, cycle: &[PathBuf]) -> String {
    let mut modules = vec![display_path(root_path, current_file)];
    modules.extend(cycle.iter().map(|path| display_path(root_path, path)));
    modules.join(" -> ")
}

fn display_path(root_path: &Path, path: &Path) -> String {
    path.strip_prefix(root_path)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{standard_library::StandardLibrary, test_util::PrettyString};
    use codespan_reporting::{
        diagnostic::Severity as CodespanSeverity, term::Config as CodespanConfig,
    };
    use std::{fs, io::Write, path::Path};

    fn lint_fixture(test_name: &'static str) -> (String, Vec<Diagnostic>) {
        let path_base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("lints")
            .join("recursive_require")
            .join(test_name);

        let lua_path = path_base.with_extension("lua");
        let lua_source = fs::read_to_string(&lua_path).expect("Cannot find lua file");
        let ast = full_moon::parse(&lua_source).expect("Cannot parse lua file");
        let ast_context = AstContext::from_ast(&ast);

        let lint = RecursiveRequireLint;
        let diagnostics = lint.pass(
            &ast,
            &Context {
                standard_library: StandardLibrary::from_name("lua51")
                    .expect("no lua51 standard library"),
                user_set_standard_library: None,
                root_path: Some(
                    path_base
                        .parent()
                        .expect("recursive_require fixture should have a parent directory")
                        .to_path_buf(),
                ),
                current_file: Some(lua_path.clone()),
            },
            &ast_context,
        );

        (lua_source, diagnostics)
    }

    fn test_fixture(test_name: &'static str) {
        let path_base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("lints")
            .join("recursive_require")
            .join(test_name);
        let (lua_source, diagnostics) = lint_fixture(test_name);

        let mut files = codespan::Files::new();
        let source_id = files.add(format!("{test_name}.lua"), lua_source);
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic.into_codespan_diagnostic(source_id, CodespanSeverity::Warning)
            })
            .collect::<Vec<_>>();

        let mut output = termcolor::NoColor::new(Vec::new());
        for diagnostic in diagnostics {
            codespan_reporting::term::emit(
                &mut output,
                &CodespanConfig::default(),
                &files,
                &diagnostic,
            )
            .expect("couldn't emit to codespan");
        }

        let stderr = std::str::from_utf8(output.get_ref()).expect("output not utf-8");
        let stderr_path = path_base.with_extension("stderr");

        if let Ok(expected) = fs::read_to_string(&stderr_path) {
            let expected = expected.replace("\r\n", "\n");
            pretty_assertions::assert_eq!(PrettyString(&expected), PrettyString(stderr));
        } else {
            let mut output_file =
                fs::File::create(stderr_path).expect("couldn't create output file");
            output_file
                .write_all(output.get_ref())
                .expect("couldn't write to output file");
        }
    }

    #[test]
    fn test_direct_cycle() {
        test_fixture("direct_cycle");
    }

    #[test]
    fn test_indirect_cycle() {
        test_fixture("indirect_cycle");
    }

    #[test]
    fn test_non_recursive_require() {
        test_fixture("non_recursive");
    }

    #[test]
    fn test_function_scope_require_is_ignored() {
        let (_, diagnostics) = lint_fixture("function_scope_ignored");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_false_and_require_is_ignored() {
        let (_, diagnostics) = lint_fixture("false_and_ignored");
        assert!(diagnostics.is_empty());
    }
}
