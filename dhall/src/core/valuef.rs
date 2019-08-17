use std::collections::HashMap;

use dhall_syntax::{
    rc, Builtin, Const, ExprF, Integer, InterpolatedTextContents, Label,
    NaiveDouble, Natural,
};

use crate::core::value::{TypedValue, Value};
use crate::core::var::{AlphaLabel, AlphaVar, Shift, Subst};
use crate::phase::normalize::{
    apply_builtin, normalize_one_layer, squash_textlit, OutputSubExpr,
};
use crate::phase::{Normalized, Typed};

/// A semantic value. The invariants ensure this value represents a Weak-Head
/// Normal Form (WHNF). This means that this first constructor is the first constructor of the
/// final Normal Form (NF).
/// This WHNF must be preserved by operations on `ValueF`s. In particular, `subst_shift` could break
/// the invariants so need to be careful to reevaluate as needed.
/// Subexpressions are Values, which are partially evaluated expressions that are normalized
/// on-demand. When all the Values in a ValueF are at least in WHNF, and recursively so, then the
/// ValueF is in NF. This is because WHNF ensures that we have the first constructor of the NF; so
/// if we have the first constructor of the NF at all levels, we actually have the NF.
/// Equality is up to alpha-equivalence (renaming of bound variables) and beta-equivalence
/// (normalization). Equality will normalize only as needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueF {
    /// Closures
    Lam(AlphaLabel, TypedValue, Value),
    Pi(AlphaLabel, TypedValue, TypedValue),
    // Invariant: the evaluation must not be able to progress further.
    AppliedBuiltin(Builtin, Vec<Value>),

    Var(AlphaVar),
    Const(Const),
    BoolLit(bool),
    NaturalLit(Natural),
    IntegerLit(Integer),
    DoubleLit(NaiveDouble),
    EmptyOptionalLit(TypedValue),
    NEOptionalLit(Value),
    // EmptyListLit(t) means `[] : List t`, not `[] : t`
    EmptyListLit(TypedValue),
    NEListLit(Vec<Value>),
    RecordLit(HashMap<Label, Value>),
    RecordType(HashMap<Label, TypedValue>),
    UnionType(HashMap<Label, Option<TypedValue>>),
    UnionConstructor(Label, HashMap<Label, Option<TypedValue>>),
    UnionLit(Label, Value, HashMap<Label, Option<TypedValue>>),
    // Invariant: this must not contain interpolations that are themselves TextLits, and
    // contiguous text values must be merged.
    TextLit(Vec<InterpolatedTextContents<Value>>),
    Equivalence(TypedValue, TypedValue),
    // Invariant: this must not contain a value captured by one of the variants above.
    PartialExpr(ExprF<Value, Normalized>),
}

impl ValueF {
    pub(crate) fn into_value(self) -> Value {
        Value::from_valuef(self)
    }

    /// Convert the value to a fully normalized syntactic expression
    pub(crate) fn normalize_to_expr(&self) -> OutputSubExpr {
        self.normalize_to_expr_maybe_alpha(false)
    }
    /// Convert the value to a fully normalized syntactic expression. Also alpha-normalize
    /// if alpha is `true`
    pub(crate) fn normalize_to_expr_maybe_alpha(
        &self,
        alpha: bool,
    ) -> OutputSubExpr {
        match self {
            ValueF::Lam(x, t, e) => rc(ExprF::Lam(
                x.to_label_maybe_alpha(alpha),
                t.normalize_to_expr_maybe_alpha(alpha),
                e.normalize_to_expr_maybe_alpha(alpha),
            )),
            ValueF::AppliedBuiltin(b, args) => {
                let mut e = rc(ExprF::Builtin(*b));
                for v in args {
                    e = rc(ExprF::App(
                        e,
                        v.normalize_to_expr_maybe_alpha(alpha),
                    ));
                }
                e
            }
            ValueF::Pi(x, t, e) => rc(ExprF::Pi(
                x.to_label_maybe_alpha(alpha),
                t.normalize_to_expr_maybe_alpha(alpha),
                e.normalize_to_expr_maybe_alpha(alpha),
            )),
            ValueF::Var(v) => rc(ExprF::Var(v.to_var(alpha))),
            ValueF::Const(c) => rc(ExprF::Const(*c)),
            ValueF::BoolLit(b) => rc(ExprF::BoolLit(*b)),
            ValueF::NaturalLit(n) => rc(ExprF::NaturalLit(*n)),
            ValueF::IntegerLit(n) => rc(ExprF::IntegerLit(*n)),
            ValueF::DoubleLit(n) => rc(ExprF::DoubleLit(*n)),
            ValueF::EmptyOptionalLit(n) => rc(ExprF::App(
                rc(ExprF::Builtin(Builtin::OptionalNone)),
                n.normalize_to_expr_maybe_alpha(alpha),
            )),
            ValueF::NEOptionalLit(n) => {
                rc(ExprF::SomeLit(n.normalize_to_expr_maybe_alpha(alpha)))
            }
            ValueF::EmptyListLit(n) => rc(ExprF::EmptyListLit(rc(ExprF::App(
                rc(ExprF::Builtin(Builtin::List)),
                n.normalize_to_expr_maybe_alpha(alpha),
            )))),
            ValueF::NEListLit(elts) => rc(ExprF::NEListLit(
                elts.iter()
                    .map(|n| n.normalize_to_expr_maybe_alpha(alpha))
                    .collect(),
            )),
            ValueF::RecordLit(kvs) => rc(ExprF::RecordLit(
                kvs.iter()
                    .map(|(k, v)| {
                        (k.clone(), v.normalize_to_expr_maybe_alpha(alpha))
                    })
                    .collect(),
            )),
            ValueF::RecordType(kts) => rc(ExprF::RecordType(
                kts.iter()
                    .map(|(k, v)| {
                        (k.clone(), v.normalize_to_expr_maybe_alpha(alpha))
                    })
                    .collect(),
            )),
            ValueF::UnionType(kts) => rc(ExprF::UnionType(
                kts.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_ref().map(|v| {
                                v.normalize_to_expr_maybe_alpha(alpha)
                            }),
                        )
                    })
                    .collect(),
            )),
            ValueF::UnionConstructor(l, kts) => {
                let kts = kts
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_ref().map(|v| {
                                v.normalize_to_expr_maybe_alpha(alpha)
                            }),
                        )
                    })
                    .collect();
                rc(ExprF::Field(rc(ExprF::UnionType(kts)), l.clone()))
            }
            ValueF::UnionLit(l, v, kts) => rc(ExprF::App(
                ValueF::UnionConstructor(l.clone(), kts.clone())
                    .normalize_to_expr_maybe_alpha(alpha),
                v.normalize_to_expr_maybe_alpha(alpha),
            )),
            ValueF::TextLit(elts) => {
                use InterpolatedTextContents::{Expr, Text};
                rc(ExprF::TextLit(
                    elts.iter()
                        .map(|contents| match contents {
                            Expr(e) => {
                                Expr(e.normalize_to_expr_maybe_alpha(alpha))
                            }
                            Text(s) => Text(s.clone()),
                        })
                        .collect(),
                ))
            }
            ValueF::Equivalence(x, y) => rc(ExprF::BinOp(
                dhall_syntax::BinOp::Equivalence,
                x.normalize_to_expr_maybe_alpha(alpha),
                y.normalize_to_expr_maybe_alpha(alpha),
            )),
            ValueF::PartialExpr(e) => {
                rc(e.map_ref(|v| v.normalize_to_expr_maybe_alpha(alpha)))
            }
        }
    }

    pub(crate) fn normalize_mut(&mut self) {
        match self {
            ValueF::Var(_)
            | ValueF::Const(_)
            | ValueF::BoolLit(_)
            | ValueF::NaturalLit(_)
            | ValueF::IntegerLit(_)
            | ValueF::DoubleLit(_) => {}

            ValueF::EmptyOptionalLit(tth) | ValueF::EmptyListLit(tth) => {
                tth.normalize_mut();
            }

            ValueF::NEOptionalLit(th) => {
                th.normalize_mut();
            }
            ValueF::Lam(_, t, e) => {
                t.normalize_mut();
                e.normalize_mut();
            }
            ValueF::Pi(_, t, e) => {
                t.normalize_mut();
                e.normalize_mut();
            }
            ValueF::AppliedBuiltin(_, args) => {
                for x in args.iter_mut() {
                    x.normalize_mut();
                }
            }
            ValueF::NEListLit(elts) => {
                for x in elts.iter_mut() {
                    x.normalize_mut();
                }
            }
            ValueF::RecordLit(kvs) => {
                for x in kvs.values_mut() {
                    x.normalize_mut();
                }
            }
            ValueF::RecordType(kvs) => {
                for x in kvs.values_mut() {
                    x.normalize_mut();
                }
            }
            ValueF::UnionType(kts) | ValueF::UnionConstructor(_, kts) => {
                for x in kts.values_mut().flat_map(|opt| opt) {
                    x.normalize_mut();
                }
            }
            ValueF::UnionLit(_, v, kts) => {
                v.normalize_mut();
                for x in kts.values_mut().flat_map(|opt| opt) {
                    x.normalize_mut();
                }
            }
            ValueF::TextLit(elts) => {
                for x in elts.iter_mut() {
                    use InterpolatedTextContents::{Expr, Text};
                    match x {
                        Expr(n) => n.normalize_mut(),
                        Text(_) => {}
                    }
                }
            }
            ValueF::Equivalence(x, y) => {
                x.normalize_mut();
                y.normalize_mut();
            }
            ValueF::PartialExpr(e) => {
                // TODO: need map_mut
                e.map_ref(|v| {
                    v.normalize_nf();
                });
            }
        }
    }

    /// Apply to a value
    pub(crate) fn app(self, val: ValueF) -> ValueF {
        self.app_valuef(val)
    }

    /// Apply to a value
    pub(crate) fn app_valuef(self, val: ValueF) -> ValueF {
        self.app_value(val.into_value())
    }

    /// Apply to a thunk
    pub fn app_value(self, th: Value) -> ValueF {
        Value::from_valuef(self).app_value(th)
    }

    pub fn from_builtin(b: Builtin) -> ValueF {
        ValueF::AppliedBuiltin(b, vec![])
    }
}

impl Shift for ValueF {
    fn shift(&self, delta: isize, var: &AlphaVar) -> Option<Self> {
        Some(match self {
            ValueF::Lam(x, t, e) => ValueF::Lam(
                x.clone(),
                t.shift(delta, var)?,
                e.shift(delta, &var.under_binder(x))?,
            ),
            ValueF::AppliedBuiltin(b, args) => {
                ValueF::AppliedBuiltin(*b, args.shift(delta, var)?)
            }
            ValueF::Pi(x, t, e) => ValueF::Pi(
                x.clone(),
                t.shift(delta, var)?,
                e.shift(delta, &var.under_binder(x))?,
            ),
            ValueF::Var(v) => ValueF::Var(v.shift(delta, var)?),
            ValueF::Const(c) => ValueF::Const(*c),
            ValueF::BoolLit(b) => ValueF::BoolLit(*b),
            ValueF::NaturalLit(n) => ValueF::NaturalLit(*n),
            ValueF::IntegerLit(n) => ValueF::IntegerLit(*n),
            ValueF::DoubleLit(n) => ValueF::DoubleLit(*n),
            ValueF::EmptyOptionalLit(tth) => {
                ValueF::EmptyOptionalLit(tth.shift(delta, var)?)
            }
            ValueF::NEOptionalLit(th) => {
                ValueF::NEOptionalLit(th.shift(delta, var)?)
            }
            ValueF::EmptyListLit(tth) => {
                ValueF::EmptyListLit(tth.shift(delta, var)?)
            }
            ValueF::NEListLit(elts) => {
                ValueF::NEListLit(elts.shift(delta, var)?)
            }
            ValueF::RecordLit(kvs) => ValueF::RecordLit(kvs.shift(delta, var)?),
            ValueF::RecordType(kvs) => {
                ValueF::RecordType(kvs.shift(delta, var)?)
            }
            ValueF::UnionType(kts) => ValueF::UnionType(kts.shift(delta, var)?),
            ValueF::UnionConstructor(x, kts) => {
                ValueF::UnionConstructor(x.clone(), kts.shift(delta, var)?)
            }
            ValueF::UnionLit(x, v, kts) => ValueF::UnionLit(
                x.clone(),
                v.shift(delta, var)?,
                kts.shift(delta, var)?,
            ),
            ValueF::TextLit(elts) => ValueF::TextLit(elts.shift(delta, var)?),
            ValueF::Equivalence(x, y) => {
                ValueF::Equivalence(x.shift(delta, var)?, y.shift(delta, var)?)
            }
            ValueF::PartialExpr(e) => ValueF::PartialExpr(e.shift(delta, var)?),
        })
    }
}

impl Subst<Typed> for ValueF {
    fn subst_shift(&self, var: &AlphaVar, val: &Typed) -> Self {
        match self {
            // Retry normalizing since substituting may allow progress
            ValueF::AppliedBuiltin(b, args) => {
                apply_builtin(*b, args.subst_shift(var, val))
            }
            // Retry normalizing since substituting may allow progress
            ValueF::PartialExpr(e) => {
                normalize_one_layer(e.subst_shift(var, val))
            }
            // Retry normalizing since substituting may allow progress
            ValueF::TextLit(elts) => ValueF::TextLit(squash_textlit(
                elts.iter().map(|contents| contents.subst_shift(var, val)),
            )),
            ValueF::Lam(x, t, e) => ValueF::Lam(
                x.clone(),
                t.subst_shift(var, val),
                e.subst_shift(&var.under_binder(x), &val.under_binder(x)),
            ),
            ValueF::Pi(x, t, e) => ValueF::Pi(
                x.clone(),
                t.subst_shift(var, val),
                e.subst_shift(&var.under_binder(x), &val.under_binder(x)),
            ),
            ValueF::Var(v) if v == var => val.to_valuef(),
            ValueF::Var(v) => ValueF::Var(v.shift(-1, var).unwrap()),
            ValueF::Const(c) => ValueF::Const(*c),
            ValueF::BoolLit(b) => ValueF::BoolLit(*b),
            ValueF::NaturalLit(n) => ValueF::NaturalLit(*n),
            ValueF::IntegerLit(n) => ValueF::IntegerLit(*n),
            ValueF::DoubleLit(n) => ValueF::DoubleLit(*n),
            ValueF::EmptyOptionalLit(tth) => {
                ValueF::EmptyOptionalLit(tth.subst_shift(var, val))
            }
            ValueF::NEOptionalLit(th) => {
                ValueF::NEOptionalLit(th.subst_shift(var, val))
            }
            ValueF::EmptyListLit(tth) => {
                ValueF::EmptyListLit(tth.subst_shift(var, val))
            }
            ValueF::NEListLit(elts) => {
                ValueF::NEListLit(elts.subst_shift(var, val))
            }
            ValueF::RecordLit(kvs) => {
                ValueF::RecordLit(kvs.subst_shift(var, val))
            }
            ValueF::RecordType(kvs) => {
                ValueF::RecordType(kvs.subst_shift(var, val))
            }
            ValueF::UnionType(kts) => {
                ValueF::UnionType(kts.subst_shift(var, val))
            }
            ValueF::UnionConstructor(x, kts) => {
                ValueF::UnionConstructor(x.clone(), kts.subst_shift(var, val))
            }
            ValueF::UnionLit(x, v, kts) => ValueF::UnionLit(
                x.clone(),
                v.subst_shift(var, val),
                kts.subst_shift(var, val),
            ),
            ValueF::Equivalence(x, y) => ValueF::Equivalence(
                x.subst_shift(var, val),
                y.subst_shift(var, val),
            ),
        }
    }
}
