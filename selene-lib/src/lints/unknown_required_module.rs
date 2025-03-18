use super::*;
use full_moon::{ast, node::Node, visitors::Visitor};
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;

#[derive(Default)]
pub struct UnknownRequiredModuleLint;

impl Lint for UnknownRequiredModuleLint {
    type Config = ();
    type Error = Infallible;

    const SEVERITY: Severity = Severity::Warning;
    const LINT_TYPE: LintType = LintType::Correctness;

    fn new(_: Self::Config) -> Result<Self, Self::Error> {
        Ok(UnknownRequiredModuleLint)
    }

    fn pass(&self, ast: &Ast, context: &Context, _ast_context: &AstContext) -> Vec<Diagnostic> {
        let mut visitor = UnknownRequiredModuleVisitor {
            positions: Vec::new(),
            root_path: context.root_path.clone(),
            current_file: context.current_file.clone(),
            local_vars: HashMap::new(),
        };

        visitor.visit_ast(ast);

        visitor
            .positions
            .into_iter()
            .map(|(start, end, module_path, path_checked)| {
                Diagnostic::new(
                    "unknown_required_module",
                    format!(
                        "could not find required module '{}' (checked path: {})",
                        module_path, path_checked
                    ),
                    Label::new((start, end)),
                )
            })
            .collect()
    }
}

struct UnknownRequiredModuleVisitor {
    positions: Vec<(u32, u32, String, String)>,
    root_path: Option<PathBuf>,
    current_file: Option<PathBuf>,
    local_vars: HashMap<String, String>,
}

impl UnknownRequiredModuleVisitor {
    /// Replaces all instances of `:WaitForChild("ModuleName")` with `.ModuleName` in a string
    fn replace_wait_for_child_calls(&self, input: &str) -> String {
        let mut processed_string = input.to_string();
        let mut search_pos = 0;

        while let Some(pos) = processed_string[search_pos..].find(":WaitForChild(") {
            let real_pos = search_pos + pos;
            // Find the closing parenthesis
            if let Some(close_pos) = processed_string[real_pos..].find(')') {
                let close_real_pos = real_pos + close_pos;

                // Extract the module name from between quotes
                let inner_content =
                    &processed_string[real_pos + ":WaitForChild(".len()..close_real_pos];

                // Remove surrounding quotes (handle both " and ' quotes)
                let module_name = inner_content
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .trim_start_matches('\'')
                    .trim_end_matches('\'');

                // Replace this instance with .ModuleName
                let replacement = format!(".{}", module_name);
                processed_string = format!(
                    "{}{}{}",
                    &processed_string[..real_pos],
                    replacement,
                    &processed_string[close_real_pos + 1..]
                );

                // Update search position to after the replacement
                search_pos = real_pos + replacement.len();
            } else {
                // No closing parenthesis found, move past this instance
                search_pos = real_pos + ":WaitForChild(".len();
            }
        }

        processed_string
    }
}

impl Visitor for UnknownRequiredModuleVisitor {
    fn visit_local_assignment(&mut self, assignment: &ast::LocalAssignment) {
        if let Some((name, expr)) = assignment
            .names()
            .iter()
            .next()
            .zip(assignment.expressions().iter().next())
        {
            let var_name = name.to_string();
            let expr_str = expr.to_string();

            // strip spaces from the variable name
            self.local_vars
                .insert(var_name.trim().to_string(), expr_str);
        }
    }

    fn visit_function_call(&mut self, call: &ast::FunctionCall) {
        if let ast::Prefix::Name(name) = call.prefix() {
            if name.to_string() == "require" {
                if let Some(ast::Suffix::Call(ast::Call::AnonymousCall(
                    ast::FunctionArgs::Parentheses { arguments, .. },
                ))) = call.suffixes().next()
                {
                    if arguments.len() == 1 {
                        // Create explicit bindings for iteration
                        let mut args_iter = arguments.iter();
                        if let Some(arg) = args_iter.next() {
                            let mut potential_logs: Vec<String> = Vec::new();
                            potential_logs.push("--------------------------------".to_string());

                            let mut arg_string = arg.to_string();

                            // convert any instances of :WaitForChild("ModuleName") to .ModuleName
                            arg_string = self.replace_wait_for_child_calls(&arg_string);

                            // if it starts with game. just remove the game. part
                            if let Some(remainder) = arg_string.strip_prefix("game.") {
                                arg_string = remainder.to_string();
                            }

                            let first_part = arg_string.split(['.', ':']).next().unwrap();
                            potential_logs.push(format!("First part: {}", first_part));
                            if first_part == "ServerScriptService"
                                || first_part == "ReplicatedStorage"
                            {
                                if let Some(root_path) = self.root_path.as_ref() {
                                    arg_string =
                                        format!("{}{}", root_path.to_string_lossy(), arg_string);
                                } else {
                                    potential_logs.push("No root path found :(".to_string());
                                }
                            } else {
                                let variable_value = self.local_vars.get(first_part);
                                potential_logs.push(format!(
                                    "Checking for variable in scope: {}",
                                    first_part
                                ));
                                if let Some(variable_value) = variable_value {
                                    potential_logs.push(format!(
                                        "Found variable in scope: {} -> {}",
                                        first_part, variable_value
                                    ));
                                    // replace the first part with the variable's value
                                    arg_string =
                                        arg_string.replace(first_part, &variable_value.to_string());
                                }
                            }

                            // @Todo(Jok): I think this is right? But worth double checking. Is script.Parent really the same thing as script?#
                            // @Todo(Jok): if it is, we could combine the two checks (one below) into one
                            // if it starts with script.Parent. replace it with the current file path
                            if let Some(remainder) = arg_string.strip_prefix("script.Parent.") {
                                // get the file's directory
                                let current_file_path =
                                    self.current_file.as_ref().unwrap().parent().unwrap();
                                arg_string = format!(
                                    "{}.{}",
                                    current_file_path.to_string_lossy(),
                                    remainder
                                );
                            }

                            // if it starts with script. replace it with the current file path
                            if let Some(remainder) = arg_string.strip_prefix("script.") {
                                // get the file's directory
                                let current_file_path =
                                    self.current_file.as_ref().unwrap().parent().unwrap();
                                arg_string = format!(
                                    "{}.{}",
                                    current_file_path.to_string_lossy(),
                                    remainder
                                );
                            }

                            // @Todo(Jok): this should only be temporary, maybe we add a config option for this?
                            // Skip packages, they're not available in the src folder
                            if arg_string.starts_with("ReplicatedStorage.Packages")
                                || arg_string.starts_with("ServerStorage.SoftRequire")
                            {
                                return;
                            }

                            // replace any remaining dots with forward slashes
                            arg_string = arg_string.replace(".", "/");

                            let mut found_file = false;

                            // if arg_string.lua exists, let's go with that
                            let lua_script_path = Path::new(&arg_string).with_extension("lua");
                            potential_logs.push(format!(
                                "lua_script_path: {}",
                                lua_script_path.to_string_lossy()
                            ));
                            if lua_script_path.exists() {
                                arg_string = lua_script_path.to_string_lossy().to_string();
                                found_file = true;
                            }

                            // if arg_string.lua doesn't exist, let's try arg_string/init.lua
                            if !found_file {
                                let init_lua_path =
                                    Path::new(&arg_string).with_file_name("init.lua");
                                potential_logs.push(format!(
                                    "Failed to find lua, checking for folder/init.lua: {}",
                                    init_lua_path.to_string_lossy()
                                ));
                                if init_lua_path.exists() {
                                    arg_string = init_lua_path.to_string_lossy().to_string();
                                    found_file = true;
                                }
                            }

                            if !found_file {
                                potential_logs.push(format!("Failed to find file for: {}", arg));
                                potential_logs.push(format!("Converted path: {}", arg_string));
                                potential_logs.push(format!(
                                    "Current file: {}",
                                    self.current_file.as_ref().unwrap().to_string_lossy()
                                ));
                            }
                            potential_logs.push("--------------------------------".to_string());

                            // output error message
                            if !found_file {
                                for _ in potential_logs {
                                    // println!("{}", log);
                                }

                                let (start, end) = arg.range().expect("call has no range");
                                self.positions.push((
                                    start.bytes() as u32,
                                    end.bytes() as u32,
                                    arg_string.clone(),
                                    arg.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
}
