use super::*;
use std::convert::Infallible;

#[cfg(feature = "roblox")]
use crate::ast_util::range;

#[cfg(feature = "roblox")]
use full_moon::{
    ast::self,
};

use full_moon::{
    ast::Ast,
    visitors::Visitor,
};

#[derive(Default)]
pub struct UnknownFunctionAttributeLint;

impl Lint for UnknownFunctionAttributeLint {
    type Config = ();
    type Error = Infallible;

    const SEVERITY: Severity = Severity::Error;
    const LINT_TYPE: LintType = LintType::Correctness;

    fn new(_: Self::Config) -> Result<Self, Self::Error> {
        Ok(UnknownFunctionAttributeLint)
    }

    fn pass(&self, ast: &Ast, context: &Context, _: &AstContext) -> Vec<Diagnostic> {
        if !context.is_roblox() {
            return Vec::new();
        }

        let mut visitor = UnknownFunctionAttributeVisitor {
            positions: Vec::new(),
        };

        visitor.visit_ast(ast);

        visitor.positions.into_iter().map(|(start, end, attr_name)| {
            Diagnostic::new(
                "unknown_function_attribute",
                format!("unknown function attribute '{}'", attr_name),
                Label::new((start, end)),
            )
        }).collect()
    }
}

struct UnknownFunctionAttributeVisitor {
    positions: Vec<(u32, u32, String)>,
}

impl Visitor for UnknownFunctionAttributeVisitor {
    #[cfg(feature = "roblox")]
    fn visit_function_declaration(&mut self, function_declaration: &ast::FunctionDeclaration) {
        for attribute in function_declaration.attributes() {
            let attr_name = attribute.name().to_string().trim().to_owned();
            if attr_name != "native" {
                self.positions.push((range(attribute).0, range(attribute).1, attr_name));
            }
        }
    }
}