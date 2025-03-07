use unicode_segmentation::UnicodeSegmentation;

use crate::diag::{bail, error, At, SourceDiagnostic, SourceResult};
use crate::eval::{destructure, ops, Eval, Vm};
use crate::foundations::{IntoValue, Value};
use crate::syntax::ast::{self, AstNode};
use crate::syntax::{Span, SyntaxKind, SyntaxNode};

/// The maximum number of loop iterations.
const MAX_ITERATIONS: usize = 10_000;

/// A control flow event that occurred during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FlowEvent {
    /// Stop iteration in a loop.
    Break(Span),
    /// Skip the remainder of the current iteration in a loop.
    Continue(Span),
    /// Stop execution of a function early, optionally returning an explicit
    /// value.
    Return(Span, Option<Value>),
}

impl FlowEvent {
    /// Return an error stating that this control flow is forbidden.
    pub fn forbidden(&self) -> SourceDiagnostic {
        match *self {
            Self::Break(span) => {
                error!(span, "cannot break outside of loop")
            }
            Self::Continue(span) => {
                error!(span, "cannot continue outside of loop")
            }
            Self::Return(span, _) => {
                error!(span, "cannot return outside of function")
            }
        }
    }
}

impl Eval for ast::Conditional<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let condition = self.condition();
        if condition.eval(vm)?.cast::<bool>().at(condition.span())? {
            self.if_body().eval(vm)
        } else if let Some(else_body) = self.else_body() {
            else_body.eval(vm)
        } else {
            Ok(Value::None)
        }
    }
}

impl Eval for ast::WhileLoop<'_> {
    type Output = Value;

    #[typst_macros::time(name = "while loop", span = self.span())]
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let flow = vm.flow.take();
        let mut output = Value::None;
        let mut i = 0;

        let condition = self.condition();
        let body = self.body();

        while condition.eval(vm)?.cast::<bool>().at(condition.span())? {
            if i == 0
                && is_invariant(condition.to_untyped())
                && !can_diverge(body.to_untyped())
            {
                bail!(condition.span(), "condition is always true");
            } else if i >= MAX_ITERATIONS {
                bail!(self.span(), "loop seems to be infinite");
            }

            let value = body.eval(vm)?;
            output = ops::join(output, value).at(body.span())?;

            match vm.flow {
                Some(FlowEvent::Break(_)) => {
                    vm.flow = None;
                    break;
                }
                Some(FlowEvent::Continue(_)) => vm.flow = None,
                Some(FlowEvent::Return(..)) => break,
                None => {}
            }

            i += 1;
        }

        if flow.is_some() {
            vm.flow = flow;
        }

        Ok(output)
    }
}

impl Eval for ast::ForLoop<'_> {
    type Output = Value;

    #[typst_macros::time(name = "for loop", span = self.span())]
    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let flow = vm.flow.take();
        let mut output = Value::None;

        macro_rules! iter {
            (for $pat:ident in $iter:expr) => {{
                vm.scopes.enter();

                #[allow(unused_parens)]
                for value in $iter {
                    destructure(vm, $pat, value.into_value())?;

                    let body = self.body();
                    let value = body.eval(vm)?;
                    output = ops::join(output, value).at(body.span())?;

                    match vm.flow {
                        Some(FlowEvent::Break(_)) => {
                            vm.flow = None;
                            break;
                        }
                        Some(FlowEvent::Continue(_)) => vm.flow = None,
                        Some(FlowEvent::Return(..)) => break,
                        None => {}
                    }
                }

                vm.scopes.exit();
            }};
        }

        let iter = self.iter().eval(vm)?;
        let pattern = self.pattern();

        match (&pattern, iter.clone()) {
            (ast::Pattern::Normal(_), Value::Str(string)) => {
                // Iterate over graphemes of string.
                iter!(for pattern in string.as_str().graphemes(true));
            }
            (_, Value::Dict(dict)) => {
                // Iterate over pairs of dict.
                iter!(for pattern in dict.pairs());
            }
            (_, Value::Array(array)) => {
                // Iterate over values of array.
                iter!(for pattern in array);
            }
            (ast::Pattern::Normal(_), _) => {
                bail!(self.iter().span(), "cannot loop over {}", iter.ty());
            }
            (_, _) => {
                bail!(pattern.span(), "cannot destructure values of {}", iter.ty())
            }
        }

        if flow.is_some() {
            vm.flow = flow;
        }

        Ok(output)
    }
}

impl Eval for ast::LoopBreak<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        if vm.flow.is_none() {
            vm.flow = Some(FlowEvent::Break(self.span()));
        }
        Ok(Value::None)
    }
}

impl Eval for ast::LoopContinue<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        if vm.flow.is_none() {
            vm.flow = Some(FlowEvent::Continue(self.span()));
        }
        Ok(Value::None)
    }
}

impl Eval for ast::FuncReturn<'_> {
    type Output = Value;

    fn eval(self, vm: &mut Vm) -> SourceResult<Self::Output> {
        let value = self.body().map(|body| body.eval(vm)).transpose()?;
        if vm.flow.is_none() {
            vm.flow = Some(FlowEvent::Return(self.span(), value));
        }
        Ok(Value::None)
    }
}

/// Whether the expression always evaluates to the same value.
fn is_invariant(expr: &SyntaxNode) -> bool {
    match expr.cast() {
        Some(ast::Expr::Ident(_)) => false,
        Some(ast::Expr::MathIdent(_)) => false,
        Some(ast::Expr::FieldAccess(access)) => {
            is_invariant(access.target().to_untyped())
        }
        Some(ast::Expr::FuncCall(call)) => {
            is_invariant(call.callee().to_untyped())
                && is_invariant(call.args().to_untyped())
        }
        _ => expr.children().all(is_invariant),
    }
}

/// Whether the expression contains a break or return.
fn can_diverge(expr: &SyntaxNode) -> bool {
    matches!(expr.kind(), SyntaxKind::Break | SyntaxKind::Return)
        || expr.children().any(can_diverge)
}
