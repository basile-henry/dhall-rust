use std::collections::HashMap;

use dhall_proc_macros as dhall;
use dhall_syntax::{
    rc, Builtin, Const, Expr, ExprF, InterpolatedTextContents, Label, Span,
    SubExpr, X,
};

use crate::core::context::{NormalizationContext, TypecheckContext};
use crate::core::thunk::{Thunk, TypeThunk};
use crate::core::value::Value;
use crate::core::var::{Shift, Subst};
use crate::error::{TypeError, TypeMessage};
use crate::phase::{Normalized, Resolved, Type, Typed};

macro_rules! ensure_equal {
    ($x:expr, $y:expr, $err:expr $(,)*) => {
        if $x.to_value() != $y.to_value() {
            return Err($err);
        }
    };
}

// Ensure the provided type has type `Type`
macro_rules! ensure_simple_type {
    ($x:expr, $err:expr $(,)*) => {{
        match $x.get_type()?.as_const() {
            Some(dhall_syntax::Const::Type) => {}
            _ => return Err($err),
        }
    }};
}

fn tck_pi_type(
    ctx: &TypecheckContext,
    x: Label,
    tx: Type,
    te: Type,
) -> Result<Typed, TypeError> {
    use crate::error::TypeMessage::*;
    let ctx2 = ctx.insert_type(&x, tx.clone());

    let ka = match tx.get_type()?.as_const() {
        Some(k) => k,
        _ => {
            return Err(TypeError::new(
                ctx,
                InvalidInputType(tx.to_normalized()),
            ))
        }
    };

    let kb = match te.get_type()?.as_const() {
        Some(k) => k,
        _ => {
            return Err(TypeError::new(
                &ctx2,
                InvalidOutputType(te.get_type()?.to_normalized()),
            ))
        }
    };

    let k = match function_check(ka, kb) {
        Ok(k) => k,
        Err(()) => {
            return Err(TypeError::new(
                ctx,
                NoDependentTypes(
                    tx.to_normalized(),
                    te.get_type()?.to_normalized(),
                ),
            ))
        }
    };

    Ok(Typed::from_thunk_and_type(
        Value::Pi(x.into(), TypeThunk::from_type(tx), TypeThunk::from_type(te))
            .into_thunk(),
        Type::from_const(k),
    ))
}

fn tck_record_type(
    ctx: &TypecheckContext,
    kts: impl IntoIterator<Item = Result<(Label, Type), TypeError>>,
) -> Result<Typed, TypeError> {
    use crate::error::TypeMessage::*;
    use std::collections::hash_map::Entry;
    let mut new_kts = HashMap::new();
    // Check that all types are the same const
    let mut k = None;
    for e in kts {
        let (x, t) = e?;
        match (k, t.get_type()?.as_const()) {
            (None, Some(k2)) => k = Some(k2),
            (Some(k1), Some(k2)) if k1 == k2 => {}
            _ => {
                return Err(TypeError::new(
                    ctx,
                    InvalidFieldType(x.clone(), t.clone()),
                ))
            }
        }
        let entry = new_kts.entry(x.clone());
        match &entry {
            Entry::Occupied(_) => {
                return Err(TypeError::new(ctx, RecordTypeDuplicateField))
            }
            Entry::Vacant(_) => {
                entry.or_insert_with(|| TypeThunk::from_type(t.clone()))
            }
        };
    }
    // An empty record type has type Type
    let k = k.unwrap_or(dhall_syntax::Const::Type);

    Ok(Typed::from_thunk_and_type(
        Value::RecordType(new_kts).into_thunk(),
        Type::from_const(k),
    ))
}

fn tck_union_type(
    ctx: &TypecheckContext,
    kts: impl IntoIterator<Item = Result<(Label, Option<Type>), TypeError>>,
) -> Result<Typed, TypeError> {
    use crate::error::TypeMessage::*;
    use std::collections::hash_map::Entry;
    let mut new_kts = HashMap::new();
    // Check that all types are the same const
    let mut k = None;
    for e in kts {
        let (x, t) = e?;
        if let Some(t) = &t {
            match (k, t.get_type()?.as_const()) {
                (None, Some(k2)) => k = Some(k2),
                (Some(k1), Some(k2)) if k1 == k2 => {}
                _ => {
                    return Err(TypeError::new(
                        ctx,
                        InvalidFieldType(x.clone(), t.clone()),
                    ))
                }
            }
        }
        let entry = new_kts.entry(x.clone());
        match &entry {
            Entry::Occupied(_) => {
                return Err(TypeError::new(ctx, UnionTypeDuplicateField))
            }
            Entry::Vacant(_) => entry.or_insert_with(|| {
                t.as_ref().map(|t| TypeThunk::from_type(t.clone()))
            }),
        };
    }

    // An empty union type has type Type;
    // an union type with only unary variants also has type Type
    let k = k.unwrap_or(dhall_syntax::Const::Type);

    Ok(Typed::from_thunk_and_type(
        Value::UnionType(new_kts).into_thunk(),
        Type::from_const(k),
    ))
}

fn tck_list_type(ctx: &TypecheckContext, t: Type) -> Result<Typed, TypeError> {
    use crate::error::TypeMessage::*;
    ensure_simple_type!(
        t,
        TypeError::new(ctx, InvalidListType(t.to_normalized())),
    );
    Ok(Typed::from_thunk_and_type(
        Value::from_builtin(Builtin::List)
            .app(t.to_value())
            .into_thunk(),
        Type::from_const(Const::Type),
    ))
}

fn tck_optional_type(
    ctx: &TypecheckContext,
    t: Type,
) -> Result<Typed, TypeError> {
    use crate::error::TypeMessage::*;
    ensure_simple_type!(
        t,
        TypeError::new(ctx, InvalidOptionalType(t.to_normalized())),
    );
    Ok(Typed::from_thunk_and_type(
        Value::from_builtin(Builtin::Optional)
            .app(t.to_value())
            .into_thunk(),
        Type::from_const(Const::Type),
    ))
}

fn function_check(a: Const, b: Const) -> Result<Const, ()> {
    use dhall_syntax::Const::*;
    match (a, b) {
        (_, Type) => Ok(Type),
        (Kind, Kind) => Ok(Kind),
        (Sort, Sort) => Ok(Sort),
        (Sort, Kind) => Ok(Sort),
        _ => Err(()),
    }
}

pub fn type_of_const(c: Const) -> Result<Type, TypeError> {
    match c {
        Const::Type => Ok(Type::from_const(Const::Kind)),
        Const::Kind => Ok(Type::from_const(Const::Sort)),
        Const::Sort => {
            Err(TypeError::new(&TypecheckContext::new(), TypeMessage::Sort))
        }
    }
}

fn type_of_builtin(b: Builtin) -> Expr<X, X> {
    use dhall_syntax::Builtin::*;
    match b {
        Bool | Natural | Integer | Double | Text => dhall::expr!(Type),
        List | Optional => dhall::expr!(
            Type -> Type
        ),

        NaturalFold => dhall::expr!(
            Natural ->
            forall (natural: Type) ->
            forall (succ: natural -> natural) ->
            forall (zero: natural) ->
            natural
        ),
        NaturalBuild => dhall::expr!(
            (forall (natural: Type) ->
                forall (succ: natural -> natural) ->
                forall (zero: natural) ->
                natural) ->
            Natural
        ),
        NaturalIsZero | NaturalEven | NaturalOdd => dhall::expr!(
            Natural -> Bool
        ),
        NaturalToInteger => dhall::expr!(Natural -> Integer),
        NaturalShow => dhall::expr!(Natural -> Text),

        IntegerToDouble => dhall::expr!(Integer -> Double),
        IntegerShow => dhall::expr!(Integer -> Text),
        DoubleShow => dhall::expr!(Double -> Text),
        TextShow => dhall::expr!(Text -> Text),

        ListBuild => dhall::expr!(
            forall (a: Type) ->
            (forall (list: Type) ->
                forall (cons: a -> list -> list) ->
                forall (nil: list) ->
                list) ->
            List a
        ),
        ListFold => dhall::expr!(
            forall (a: Type) ->
            List a ->
            forall (list: Type) ->
            forall (cons: a -> list -> list) ->
            forall (nil: list) ->
            list
        ),
        ListLength => dhall::expr!(forall (a: Type) -> List a -> Natural),
        ListHead | ListLast => {
            dhall::expr!(forall (a: Type) -> List a -> Optional a)
        }
        ListIndexed => dhall::expr!(
            forall (a: Type) ->
            List a ->
            List { index: Natural, value: a }
        ),
        ListReverse => dhall::expr!(
            forall (a: Type) -> List a -> List a
        ),

        OptionalBuild => dhall::expr!(
            forall (a: Type) ->
            (forall (optional: Type) ->
                forall (just: a -> optional) ->
                forall (nothing: optional) ->
                optional) ->
            Optional a
        ),
        OptionalFold => dhall::expr!(
            forall (a: Type) ->
            Optional a ->
            forall (optional: Type) ->
            forall (just: a -> optional) ->
            forall (nothing: optional) ->
            optional
        ),
        OptionalNone => dhall::expr!(
            forall (a: Type) -> Optional a
        ),
    }
}

/// Takes an expression that is meant to contain a Type
/// and turn it into a type, typechecking it along the way.
pub fn mktype(
    ctx: &TypecheckContext,
    e: SubExpr<Span, Normalized>,
) -> Result<Type, TypeError> {
    Ok(type_with(ctx, e)?.to_type())
}

pub fn builtin_to_type(b: Builtin) -> Result<Type, TypeError> {
    mktype(&TypecheckContext::new(), SubExpr::from_builtin(b))
}

/// Intermediary return type
enum Ret {
    /// Returns the contained value as is
    RetWhole(Typed),
    /// Use the contained Type as the type of the input expression
    RetTypeOnly(Type),
}

/// Type-check an expression and return the expression alongside its type if type-checking
/// succeeded, or an error if type-checking failed.
/// Some normalization is done while typechecking, so the returned expression might be partially
/// normalized as well.
fn type_with(
    ctx: &TypecheckContext,
    e: SubExpr<Span, Normalized>,
) -> Result<Typed, TypeError> {
    use dhall_syntax::ExprF::{
        Annot, App, Embed, Lam, Let, OldOptionalLit, Pi, SomeLit, Var,
    };

    use Ret::*;
    Ok(match e.as_ref() {
        Lam(x, t, b) => {
            let tx = mktype(ctx, t.clone())?;
            let ctx2 = ctx.insert_type(x, tx.clone());
            let b = type_with(&ctx2, b.clone())?;
            let v = Value::Lam(
                x.clone().into(),
                TypeThunk::from_type(tx.clone()),
                b.to_thunk(),
            );
            let tb = b.get_type()?.into_owned();
            let t = tck_pi_type(ctx, x.clone(), tx, tb)?.to_type();
            Typed::from_thunk_and_type(Thunk::from_value(v), t)
        }
        Pi(x, ta, tb) => {
            let ta = mktype(ctx, ta.clone())?;
            let ctx2 = ctx.insert_type(x, ta.clone());
            let tb = mktype(&ctx2, tb.clone())?;
            return tck_pi_type(ctx, x.clone(), ta, tb);
        }
        Let(x, t, v, e) => {
            let v = if let Some(t) = t {
                t.rewrap(Annot(v.clone(), t.clone()))
            } else {
                v.clone()
            };

            let v = type_with(ctx, v)?;
            return type_with(&ctx.insert_value(x, v.clone())?, e.clone());
        }
        OldOptionalLit(None, t) => {
            let none = SubExpr::from_builtin(Builtin::OptionalNone);
            let e = e.rewrap(App(none, t.clone()));
            return type_with(ctx, e);
        }
        OldOptionalLit(Some(x), t) => {
            let optional = SubExpr::from_builtin(Builtin::Optional);
            let x = x.rewrap(SomeLit(x.clone()));
            let t = t.rewrap(App(optional, t.clone()));
            let e = e.rewrap(Annot(x, t));
            return type_with(ctx, e);
        }
        Embed(p) => p.clone().into_typed(),
        Var(var) => match ctx.lookup(&var) {
            Some(typed) => typed,
            None => {
                return Err(TypeError::new(
                    ctx,
                    TypeMessage::UnboundVariable(var.clone()),
                ))
            }
        },
        _ => {
            // Typecheck recursively all subexpressions
            let expr =
                e.as_ref().traverse_ref_with_special_handling_of_binders(
                    |e| type_with(ctx, e.clone()),
                    |_, _| unreachable!(),
                    |_| unreachable!(),
                )?;
            let ret = type_last_layer(ctx, &expr)?;
            match ret {
                RetTypeOnly(typ) => {
                    let expr = expr.map_ref_simple(|typed| typed.to_thunk());
                    Typed::from_thunk_and_type(
                        Thunk::from_partial_expr(expr),
                        typ,
                    )
                }
                RetWhole(tt) => tt,
            }
        }
    })
}

/// When all sub-expressions have been typed, check the remaining toplevel
/// layer.
fn type_last_layer(
    ctx: &TypecheckContext,
    e: &ExprF<Typed, X>,
) -> Result<Ret, TypeError> {
    use crate::error::TypeMessage::*;
    use dhall_syntax::BinOp::*;
    use dhall_syntax::Builtin::*;
    use dhall_syntax::ExprF::*;
    use Ret::*;
    let mkerr = |msg: TypeMessage| TypeError::new(ctx, msg);

    match e {
        Lam(_, _, _)
        | Pi(_, _, _)
        | Let(_, _, _, _)
        | OldOptionalLit(_, _)
        | Embed(_)
        | Var(_) => unreachable!(),
        App(f, a) => {
            let tf = f.get_type()?;
            let (x, tx, tb) = match &tf.to_value() {
                Value::Pi(x, tx, tb) => (x.clone(), tx.to_type(), tb.to_type()),
                _ => return Err(mkerr(NotAFunction(f.clone()))),
            };
            ensure_equal!(a.get_type()?, &tx, {
                mkerr(TypeMismatch(f.clone(), tx.to_normalized(), a.clone()))
            });

            Ok(RetTypeOnly(tb.subst_shift(&x.into(), &a)))
        }
        Annot(x, t) => {
            let t = t.to_type();
            ensure_equal!(
                &t,
                x.get_type()?,
                mkerr(AnnotMismatch(x.clone(), t.to_normalized()))
            );
            Ok(RetTypeOnly(x.get_type()?.into_owned()))
        }
        BoolIf(x, y, z) => {
            ensure_equal!(
                x.get_type()?,
                &builtin_to_type(Bool)?,
                mkerr(InvalidPredicate(x.clone())),
            );

            ensure_simple_type!(
                y.get_type()?,
                mkerr(IfBranchMustBeTerm(true, y.clone())),
            );

            ensure_simple_type!(
                z.get_type()?,
                mkerr(IfBranchMustBeTerm(false, z.clone())),
            );

            ensure_equal!(
                y.get_type()?,
                z.get_type()?,
                mkerr(IfBranchMismatch(y.clone(), z.clone()))
            );

            Ok(RetTypeOnly(y.get_type()?.into_owned()))
        }
        EmptyListLit(t) => {
            let t = t.to_type();
            Ok(RetTypeOnly(tck_list_type(ctx, t)?.to_type()))
        }
        NEListLit(xs) => {
            let mut iter = xs.iter().enumerate();
            let (_, x) = iter.next().unwrap();
            for (i, y) in iter {
                ensure_equal!(
                    x.get_type()?,
                    y.get_type()?,
                    mkerr(InvalidListElement(
                        i,
                        x.get_type()?.to_normalized(),
                        y.clone()
                    ))
                );
            }
            let t = x.get_type()?.into_owned();
            Ok(RetTypeOnly(tck_list_type(ctx, t)?.to_type()))
        }
        SomeLit(x) => {
            let t = x.get_type()?.into_owned();
            Ok(RetTypeOnly(tck_optional_type(ctx, t)?.to_type()))
        }
        RecordType(kts) => Ok(RetWhole(tck_record_type(
            ctx,
            kts.iter().map(|(x, t)| Ok((x.clone(), t.to_type()))),
        )?)),
        UnionType(kts) => Ok(RetWhole(tck_union_type(
            ctx,
            kts.iter()
                .map(|(x, t)| Ok((x.clone(), t.as_ref().map(|t| t.to_type())))),
        )?)),
        RecordLit(kvs) => Ok(RetTypeOnly(
            tck_record_type(
                ctx,
                kvs.iter()
                    .map(|(x, v)| Ok((x.clone(), v.get_type()?.into_owned()))),
            )?
            .into_type(),
        )),
        UnionLit(x, v, kvs) => {
            use std::iter::once;
            let kts = kvs
                .iter()
                .map(|(x, v)| Ok((x.clone(), v.as_ref().map(|v| v.to_type()))));
            let t = v.get_type()?.into_owned();
            let kts = kts.chain(once(Ok((x.clone(), Some(t)))));
            Ok(RetTypeOnly(tck_union_type(ctx, kts)?.to_type()))
        }
        Field(r, x) => {
            match &r.get_type()?.to_value() {
                Value::RecordType(kts) => match kts.get(&x) {
                    Some(tth) => {
                        Ok(RetTypeOnly(tth.to_type()))
                    },
                    None => Err(mkerr(MissingRecordField(x.clone(),
                                        r.clone()))),
                },
                // TODO: branch here only when r.get_type() is a Const
                _ => {
                    let r = r.to_type();
                    match &r.to_value() {
                        Value::UnionType(kts) => match kts.get(&x) {
                            // Constructor has type T -> < x: T, ... >
                            Some(Some(t)) => {
                                // TODO: avoid capture
                                Ok(RetTypeOnly(
                                    tck_pi_type(
                                        ctx,
                                        "_".into(),
                                        t.to_type(),
                                        r.clone(),
                                    )?.to_type()
                                ))
                            },
                            Some(None) => {
                                Ok(RetTypeOnly(r.clone()))
                            },
                            None => {
                                Err(mkerr(MissingUnionField(
                                    x.clone(),
                                    r.to_normalized(),
                                )))
                            },
                        },
                        _ => {
                            Err(mkerr(NotARecord(
                                x.clone(),
                                r.to_normalized()
                            )))
                        },
                    }
                }
                // _ => Err(mkerr(NotARecord(
                //     x,
                //     r.to_type()?.to_normalized(),
                // ))),
            }
        }
        Const(c) => Ok(RetWhole(Typed::from_const(*c))),
        Builtin(b) => {
            Ok(RetTypeOnly(mktype(ctx, rc(type_of_builtin(*b)).absurd())?))
        }
        BoolLit(_) => Ok(RetTypeOnly(builtin_to_type(Bool)?)),
        NaturalLit(_) => Ok(RetTypeOnly(builtin_to_type(Natural)?)),
        IntegerLit(_) => Ok(RetTypeOnly(builtin_to_type(Integer)?)),
        DoubleLit(_) => Ok(RetTypeOnly(builtin_to_type(Double)?)),
        TextLit(interpolated) => {
            let text_type = builtin_to_type(Text)?;
            for contents in interpolated.iter() {
                use InterpolatedTextContents::Expr;
                if let Expr(x) = contents {
                    ensure_equal!(
                        x.get_type()?,
                        &text_type,
                        mkerr(InvalidTextInterpolation(x.clone())),
                    );
                }
            }
            Ok(RetTypeOnly(text_type))
        }
        BinOp(o @ ListAppend, l, r) => {
            match l.get_type()?.to_value() {
                Value::AppliedBuiltin(List, _) => {}
                _ => return Err(mkerr(BinOpTypeMismatch(*o, l.clone()))),
            }

            ensure_equal!(
                l.get_type()?,
                r.get_type()?,
                mkerr(BinOpTypeMismatch(*o, r.clone()))
            );

            Ok(RetTypeOnly(l.get_type()?.into_owned()))
        }
        BinOp(o @ RecursiveRecordMerge, l, r) => {
            let l_type = l.get_type()?;
            let r_type = r.get_type()?;

            match (l_type.to_value(), r_type.to_value()) {
                (Value::RecordLit(l_kvs), Value::RecordLit(r_kvs)) => {
                    use crate::phase::normalize::merge_maps;
                    use crate::api::Type;
                    let kvs = merge_maps(&l_kvs, &r_kvs, |_, r_t| r_t.clone());

                    Ok(RetTypeOnly(Type::make_record_type(kvs.iter())))
                }
                _ => Err(mkerr(BinOpTypeMismatch(*o, l.clone()))),
            }
        }
        BinOp(o, l, r) => {
            let t = builtin_to_type(match o {
                BoolAnd => Bool,
                BoolOr => Bool,
                BoolEQ => Bool,
                BoolNE => Bool,
                NaturalPlus => Natural,
                NaturalTimes => Natural,
                TextAppend => Text,
                ListAppend => unreachable!(),
                RecursiveRecordMerge => unreachable!(),
                _ => return Err(mkerr(Unimplemented)),
            })?;

            ensure_equal!(
                l.get_type()?,
                &t,
                mkerr(BinOpTypeMismatch(*o, l.clone()))
            );

            ensure_equal!(
                r.get_type()?,
                &t,
                mkerr(BinOpTypeMismatch(*o, r.clone()))
            );

            Ok(RetTypeOnly(t))
        }
        Merge(record, union, type_annot) => {
            let handlers = match record.get_type()?.to_value() {
                Value::RecordType(kts) => kts,
                _ => return Err(mkerr(Merge1ArgMustBeRecord(record.clone()))),
            };

            let variants = match union.get_type()?.to_value() {
                Value::UnionType(kts) => kts,
                _ => return Err(mkerr(Merge2ArgMustBeUnion(union.clone()))),
            };

            let mut inferred_type = None;
            for (x, handler) in handlers.iter() {
                let handler_return_type = match variants.get(x) {
                    // Union alternative with type
                    Some(Some(variant_type)) => {
                        let variant_type = variant_type.to_type();
                        let handler_type = handler.to_type();
                        let (x, tx, tb) = match &handler_type.to_value() {
                            Value::Pi(x, tx, tb) => {
                                (x.clone(), tx.to_type(), tb.to_type())
                            }
                            _ => return Err(mkerr(NotAFunction(handler_type))),
                        };

                        ensure_equal!(&variant_type, &tx, {
                            mkerr(TypeMismatch(
                                handler_type,
                                tx.to_normalized(),
                                variant_type,
                            ))
                        });

                        // Extract `tb` from under the `x` binder. Fails is `x` was free in `tb`.
                        match tb.over_binder(x) {
                            Some(x) => x,
                            None => {
                                return Err(mkerr(
                                    MergeHandlerReturnTypeMustNotBeDependent,
                                ))
                            }
                        }
                    }
                    // Union alternative without type
                    Some(None) => handler.to_type(),
                    None => {
                        return Err(mkerr(MergeHandlerMissingVariant(
                            x.clone(),
                        )))
                    }
                };
                match &inferred_type {
                    None => inferred_type = Some(handler_return_type),
                    Some(t) => {
                        ensure_equal!(
                            t,
                            &handler_return_type,
                            mkerr(MergeHandlerTypeMismatch)
                        );
                    }
                }
            }
            for x in variants.keys() {
                if !handlers.contains_key(x) {
                    return Err(mkerr(MergeVariantMissingHandler(x.clone())));
                }
            }

            match (inferred_type, type_annot) {
                (Some(ref t1), Some(t2)) => {
                    let t2 = t2.to_type();
                    ensure_equal!(t1, &t2, mkerr(MergeAnnotMismatch));
                    Ok(RetTypeOnly(t2))
                }
                (Some(t), None) => Ok(RetTypeOnly(t)),
                (None, Some(t)) => Ok(RetTypeOnly(t.to_type())),
                (None, None) => Err(mkerr(MergeEmptyNeedsAnnotation)),
            }
        }
        Projection(record, labels) => {
            let trecord = record.get_type()?;
            let kts = match trecord.to_value() {
                Value::RecordType(kts) => kts,
                _ => return Err(mkerr(ProjectionMustBeRecord)),
            };

            let mut new_kts = HashMap::new();
            for l in labels {
                match kts.get(l) {
                    None => return Err(mkerr(ProjectionMissingEntry)),
                    Some(t) => new_kts.insert(l.clone(), t.clone()),
                };
            }

            Ok(RetTypeOnly(
                Typed::from_thunk_and_type(
                    Value::RecordType(new_kts).into_thunk(),
                    trecord.get_type()?.into_owned(),
                )
                .to_type(),
            ))
        }
    }
}

/// `typeOf` is the same as `type_with` with an empty context, meaning that the
/// expression must be closed (i.e. no free variables), otherwise type-checking
/// will fail.
fn type_of(e: SubExpr<Span, Normalized>) -> Result<Typed, TypeError> {
    let ctx = TypecheckContext::new();
    let e = type_with(&ctx, e)?;
    // Ensure `e` has a type (i.e. `e` is not `Sort`)
    e.get_type()?;
    Ok(e)
}

pub fn typecheck(e: Resolved) -> Result<Typed, TypeError> {
    type_of(e.0)
}

pub fn typecheck_with(e: Resolved, ty: &Type) -> Result<Typed, TypeError> {
    let expr: SubExpr<_, _> = e.0;
    let ty: SubExpr<_, _> = ty.to_expr().absurd();
    type_of(expr.rewrap(ExprF::Annot(expr.clone(), ty)))
}
pub fn skip_typecheck(e: Resolved) -> Typed {
    Typed::from_thunk_untyped(Thunk::new(NormalizationContext::new(), e.0))
}

#[cfg(test)]
mod spec_tests {
    #![rustfmt::skip]

    macro_rules! tc_success {
        ($name:ident, $path:expr) => {
            make_spec_test!(Typecheck, Success, $name, &("success/".to_owned() + $path));
        };
    }
    macro_rules! tc_failure {
        ($name:ident, $path:expr) => {
            make_spec_test!(Typecheck, Failure, $name, &("failure/".to_owned() + $path));
        };
    }

    macro_rules! ti_success {
        ($name:ident, $path:expr) => {
            make_spec_test!(TypeInference, Success, $name, &("success/".to_owned() + $path));
        };
    }
    // macro_rules! ti_failure {
    //     ($name:ident, $path:expr) => {
    //         make_spec_test!(TypeInference, Failure, $name, &("failure/".to_owned() + $path));
    //     };
    // }

    // tc_success!(tc_success_accessEncodedType, "accessEncodedType");
    // tc_success!(tc_success_accessType, "accessType");
    tc_success!(tc_success_prelude_Bool_and_0, "prelude/Bool/and/0");
    tc_success!(tc_success_prelude_Bool_and_1, "prelude/Bool/and/1");
    tc_success!(tc_success_prelude_Bool_build_0, "prelude/Bool/build/0");
    tc_success!(tc_success_prelude_Bool_build_1, "prelude/Bool/build/1");
    tc_success!(tc_success_prelude_Bool_even_0, "prelude/Bool/even/0");
    tc_success!(tc_success_prelude_Bool_even_1, "prelude/Bool/even/1");
    tc_success!(tc_success_prelude_Bool_even_2, "prelude/Bool/even/2");
    tc_success!(tc_success_prelude_Bool_even_3, "prelude/Bool/even/3");
    tc_success!(tc_success_prelude_Bool_fold_0, "prelude/Bool/fold/0");
    tc_success!(tc_success_prelude_Bool_fold_1, "prelude/Bool/fold/1");
    tc_success!(tc_success_prelude_Bool_not_0, "prelude/Bool/not/0");
    tc_success!(tc_success_prelude_Bool_not_1, "prelude/Bool/not/1");
    tc_success!(tc_success_prelude_Bool_odd_0, "prelude/Bool/odd/0");
    tc_success!(tc_success_prelude_Bool_odd_1, "prelude/Bool/odd/1");
    tc_success!(tc_success_prelude_Bool_odd_2, "prelude/Bool/odd/2");
    tc_success!(tc_success_prelude_Bool_odd_3, "prelude/Bool/odd/3");
    tc_success!(tc_success_prelude_Bool_or_0, "prelude/Bool/or/0");
    tc_success!(tc_success_prelude_Bool_or_1, "prelude/Bool/or/1");
    tc_success!(tc_success_prelude_Bool_show_0, "prelude/Bool/show/0");
    tc_success!(tc_success_prelude_Bool_show_1, "prelude/Bool/show/1");
    tc_success!(tc_success_prelude_Double_show_0, "prelude/Double/show/0");
    tc_success!(tc_success_prelude_Double_show_1, "prelude/Double/show/1");
    tc_success!(tc_success_prelude_Integer_show_0, "prelude/Integer/show/0");
    tc_success!(tc_success_prelude_Integer_show_1, "prelude/Integer/show/1");
    tc_success!(tc_success_prelude_Integer_toDouble_0, "prelude/Integer/toDouble/0");
    tc_success!(tc_success_prelude_Integer_toDouble_1, "prelude/Integer/toDouble/1");
    tc_success!(tc_success_prelude_List_all_0, "prelude/List/all/0");
    tc_success!(tc_success_prelude_List_all_1, "prelude/List/all/1");
    tc_success!(tc_success_prelude_List_any_0, "prelude/List/any/0");
    tc_success!(tc_success_prelude_List_any_1, "prelude/List/any/1");
    tc_success!(tc_success_prelude_List_build_0, "prelude/List/build/0");
    tc_success!(tc_success_prelude_List_build_1, "prelude/List/build/1");
    tc_success!(tc_success_prelude_List_concat_0, "prelude/List/concat/0");
    tc_success!(tc_success_prelude_List_concat_1, "prelude/List/concat/1");
    tc_success!(tc_success_prelude_List_concatMap_0, "prelude/List/concatMap/0");
    tc_success!(tc_success_prelude_List_concatMap_1, "prelude/List/concatMap/1");
    tc_success!(tc_success_prelude_List_filter_0, "prelude/List/filter/0");
    tc_success!(tc_success_prelude_List_filter_1, "prelude/List/filter/1");
    tc_success!(tc_success_prelude_List_fold_0, "prelude/List/fold/0");
    tc_success!(tc_success_prelude_List_fold_1, "prelude/List/fold/1");
    tc_success!(tc_success_prelude_List_fold_2, "prelude/List/fold/2");
    tc_success!(tc_success_prelude_List_generate_0, "prelude/List/generate/0");
    tc_success!(tc_success_prelude_List_generate_1, "prelude/List/generate/1");
    tc_success!(tc_success_prelude_List_head_0, "prelude/List/head/0");
    tc_success!(tc_success_prelude_List_head_1, "prelude/List/head/1");
    tc_success!(tc_success_prelude_List_indexed_0, "prelude/List/indexed/0");
    tc_success!(tc_success_prelude_List_indexed_1, "prelude/List/indexed/1");
    tc_success!(tc_success_prelude_List_iterate_0, "prelude/List/iterate/0");
    tc_success!(tc_success_prelude_List_iterate_1, "prelude/List/iterate/1");
    tc_success!(tc_success_prelude_List_last_0, "prelude/List/last/0");
    tc_success!(tc_success_prelude_List_last_1, "prelude/List/last/1");
    tc_success!(tc_success_prelude_List_length_0, "prelude/List/length/0");
    tc_success!(tc_success_prelude_List_length_1, "prelude/List/length/1");
    tc_success!(tc_success_prelude_List_map_0, "prelude/List/map/0");
    tc_success!(tc_success_prelude_List_map_1, "prelude/List/map/1");
    tc_success!(tc_success_prelude_List_null_0, "prelude/List/null/0");
    tc_success!(tc_success_prelude_List_null_1, "prelude/List/null/1");
    tc_success!(tc_success_prelude_List_replicate_0, "prelude/List/replicate/0");
    tc_success!(tc_success_prelude_List_replicate_1, "prelude/List/replicate/1");
    tc_success!(tc_success_prelude_List_reverse_0, "prelude/List/reverse/0");
    tc_success!(tc_success_prelude_List_reverse_1, "prelude/List/reverse/1");
    tc_success!(tc_success_prelude_List_shifted_0, "prelude/List/shifted/0");
    tc_success!(tc_success_prelude_List_shifted_1, "prelude/List/shifted/1");
    tc_success!(tc_success_prelude_List_unzip_0, "prelude/List/unzip/0");
    tc_success!(tc_success_prelude_List_unzip_1, "prelude/List/unzip/1");
    tc_success!(tc_success_prelude_Monoid_00, "prelude/Monoid/00");
    tc_success!(tc_success_prelude_Monoid_01, "prelude/Monoid/01");
    tc_success!(tc_success_prelude_Monoid_02, "prelude/Monoid/02");
    tc_success!(tc_success_prelude_Monoid_03, "prelude/Monoid/03");
    tc_success!(tc_success_prelude_Monoid_04, "prelude/Monoid/04");
    tc_success!(tc_success_prelude_Monoid_05, "prelude/Monoid/05");
    tc_success!(tc_success_prelude_Monoid_06, "prelude/Monoid/06");
    tc_success!(tc_success_prelude_Monoid_07, "prelude/Monoid/07");
    tc_success!(tc_success_prelude_Monoid_08, "prelude/Monoid/08");
    tc_success!(tc_success_prelude_Monoid_09, "prelude/Monoid/09");
    tc_success!(tc_success_prelude_Monoid_10, "prelude/Monoid/10");
    tc_success!(tc_success_prelude_Natural_build_0, "prelude/Natural/build/0");
    tc_success!(tc_success_prelude_Natural_build_1, "prelude/Natural/build/1");
    tc_success!(tc_success_prelude_Natural_enumerate_0, "prelude/Natural/enumerate/0");
    tc_success!(tc_success_prelude_Natural_enumerate_1, "prelude/Natural/enumerate/1");
    tc_success!(tc_success_prelude_Natural_even_0, "prelude/Natural/even/0");
    tc_success!(tc_success_prelude_Natural_even_1, "prelude/Natural/even/1");
    tc_success!(tc_success_prelude_Natural_fold_0, "prelude/Natural/fold/0");
    tc_success!(tc_success_prelude_Natural_fold_1, "prelude/Natural/fold/1");
    tc_success!(tc_success_prelude_Natural_fold_2, "prelude/Natural/fold/2");
    tc_success!(tc_success_prelude_Natural_isZero_0, "prelude/Natural/isZero/0");
    tc_success!(tc_success_prelude_Natural_isZero_1, "prelude/Natural/isZero/1");
    tc_success!(tc_success_prelude_Natural_odd_0, "prelude/Natural/odd/0");
    tc_success!(tc_success_prelude_Natural_odd_1, "prelude/Natural/odd/1");
    tc_success!(tc_success_prelude_Natural_product_0, "prelude/Natural/product/0");
    tc_success!(tc_success_prelude_Natural_product_1, "prelude/Natural/product/1");
    tc_success!(tc_success_prelude_Natural_show_0, "prelude/Natural/show/0");
    tc_success!(tc_success_prelude_Natural_show_1, "prelude/Natural/show/1");
    tc_success!(tc_success_prelude_Natural_sum_0, "prelude/Natural/sum/0");
    tc_success!(tc_success_prelude_Natural_sum_1, "prelude/Natural/sum/1");
    tc_success!(tc_success_prelude_Natural_toDouble_0, "prelude/Natural/toDouble/0");
    tc_success!(tc_success_prelude_Natural_toDouble_1, "prelude/Natural/toDouble/1");
    tc_success!(tc_success_prelude_Natural_toInteger_0, "prelude/Natural/toInteger/0");
    tc_success!(tc_success_prelude_Natural_toInteger_1, "prelude/Natural/toInteger/1");
    tc_success!(tc_success_prelude_Optional_all_0, "prelude/Optional/all/0");
    tc_success!(tc_success_prelude_Optional_all_1, "prelude/Optional/all/1");
    tc_success!(tc_success_prelude_Optional_any_0, "prelude/Optional/any/0");
    tc_success!(tc_success_prelude_Optional_any_1, "prelude/Optional/any/1");
    tc_success!(tc_success_prelude_Optional_build_0, "prelude/Optional/build/0");
    tc_success!(tc_success_prelude_Optional_build_1, "prelude/Optional/build/1");
    tc_success!(tc_success_prelude_Optional_concat_0, "prelude/Optional/concat/0");
    tc_success!(tc_success_prelude_Optional_concat_1, "prelude/Optional/concat/1");
    tc_success!(tc_success_prelude_Optional_concat_2, "prelude/Optional/concat/2");
    tc_success!(tc_success_prelude_Optional_filter_0, "prelude/Optional/filter/0");
    tc_success!(tc_success_prelude_Optional_filter_1, "prelude/Optional/filter/1");
    tc_success!(tc_success_prelude_Optional_fold_0, "prelude/Optional/fold/0");
    tc_success!(tc_success_prelude_Optional_fold_1, "prelude/Optional/fold/1");
    tc_success!(tc_success_prelude_Optional_head_0, "prelude/Optional/head/0");
    tc_success!(tc_success_prelude_Optional_head_1, "prelude/Optional/head/1");
    tc_success!(tc_success_prelude_Optional_head_2, "prelude/Optional/head/2");
    tc_success!(tc_success_prelude_Optional_last_0, "prelude/Optional/last/0");
    tc_success!(tc_success_prelude_Optional_last_1, "prelude/Optional/last/1");
    tc_success!(tc_success_prelude_Optional_last_2, "prelude/Optional/last/2");
    tc_success!(tc_success_prelude_Optional_length_0, "prelude/Optional/length/0");
    tc_success!(tc_success_prelude_Optional_length_1, "prelude/Optional/length/1");
    tc_success!(tc_success_prelude_Optional_map_0, "prelude/Optional/map/0");
    tc_success!(tc_success_prelude_Optional_map_1, "prelude/Optional/map/1");
    tc_success!(tc_success_prelude_Optional_null_0, "prelude/Optional/null/0");
    tc_success!(tc_success_prelude_Optional_null_1, "prelude/Optional/null/1");
    tc_success!(tc_success_prelude_Optional_toList_0, "prelude/Optional/toList/0");
    tc_success!(tc_success_prelude_Optional_toList_1, "prelude/Optional/toList/1");
    tc_success!(tc_success_prelude_Optional_unzip_0, "prelude/Optional/unzip/0");
    tc_success!(tc_success_prelude_Optional_unzip_1, "prelude/Optional/unzip/1");
    tc_success!(tc_success_prelude_Text_concat_0, "prelude/Text/concat/0");
    tc_success!(tc_success_prelude_Text_concat_1, "prelude/Text/concat/1");
    tc_success!(tc_success_prelude_Text_concatMap_0, "prelude/Text/concatMap/0");
    tc_success!(tc_success_prelude_Text_concatMap_1, "prelude/Text/concatMap/1");
    // tc_success!(tc_success_prelude_Text_concatMapSep_0, "prelude/Text/concatMapSep/0");
    // tc_success!(tc_success_prelude_Text_concatMapSep_1, "prelude/Text/concatMapSep/1");
    // tc_success!(tc_success_prelude_Text_concatSep_0, "prelude/Text/concatSep/0");
    // tc_success!(tc_success_prelude_Text_concatSep_1, "prelude/Text/concatSep/1");
    tc_success!(tc_success_recordOfRecordOfTypes, "recordOfRecordOfTypes");
    tc_success!(tc_success_recordOfTypes, "recordOfTypes");
    // tc_success!(tc_success_simple_access_0, "simple/access/0");
    // tc_success!(tc_success_simple_access_1, "simple/access/1");
    // tc_success!(tc_success_simple_alternativesAreTypes, "simple/alternativesAreTypes");
    // tc_success!(tc_success_simple_anonymousFunctionsInTypes, "simple/anonymousFunctionsInTypes");
    // tc_success!(tc_success_simple_fieldsAreTypes, "simple/fieldsAreTypes");
    // tc_success!(tc_success_simple_kindParameter, "simple/kindParameter");
    tc_success!(tc_success_simple_mergeEquivalence, "simple/mergeEquivalence");
    // tc_success!(tc_success_simple_mixedFieldAccess, "simple/mixedFieldAccess");
    tc_success!(tc_success_simple_unionsOfTypes, "simple/unionsOfTypes");

    tc_failure!(tc_failure_combineMixedRecords, "combineMixedRecords");
    tc_failure!(tc_failure_duplicateFields, "duplicateFields");
    tc_failure!(tc_failure_hurkensParadox, "hurkensParadox");
    // tc_failure!(tc_failure_importBoundary, "importBoundary");
    tc_failure!(tc_failure_mixedUnions, "mixedUnions");
    tc_failure!(tc_failure_preferMixedRecords, "preferMixedRecords");
    tc_failure!(tc_failure_unit_FunctionApplicationArgumentNotMatch, "unit/FunctionApplicationArgumentNotMatch");
    tc_failure!(tc_failure_unit_FunctionApplicationIsNotFunction, "unit/FunctionApplicationIsNotFunction");
    tc_failure!(tc_failure_unit_FunctionArgumentTypeNotAType, "unit/FunctionArgumentTypeNotAType");
    tc_failure!(tc_failure_unit_FunctionDependentType, "unit/FunctionDependentType");
    tc_failure!(tc_failure_unit_FunctionDependentType2, "unit/FunctionDependentType2");
    tc_failure!(tc_failure_unit_FunctionTypeArgumentTypeNotAType, "unit/FunctionTypeArgumentTypeNotAType");
    tc_failure!(tc_failure_unit_FunctionTypeKindSort, "unit/FunctionTypeKindSort");
    tc_failure!(tc_failure_unit_FunctionTypeTypeKind, "unit/FunctionTypeTypeKind");
    tc_failure!(tc_failure_unit_FunctionTypeTypeSort, "unit/FunctionTypeTypeSort");
    tc_failure!(tc_failure_unit_IfBranchesNotMatch, "unit/IfBranchesNotMatch");
    tc_failure!(tc_failure_unit_IfBranchesNotType, "unit/IfBranchesNotType");
    tc_failure!(tc_failure_unit_IfNotBool, "unit/IfNotBool");
    tc_failure!(tc_failure_unit_LetWithWrongAnnotation, "unit/LetWithWrongAnnotation");
    tc_failure!(tc_failure_unit_ListLiteralEmptyNotType, "unit/ListLiteralEmptyNotType");
    tc_failure!(tc_failure_unit_ListLiteralNotType, "unit/ListLiteralNotType");
    tc_failure!(tc_failure_unit_ListLiteralTypesNotMatch, "unit/ListLiteralTypesNotMatch");
    tc_failure!(tc_failure_unit_MergeAlternativeHasNoHandler, "unit/MergeAlternativeHasNoHandler");
    tc_failure!(tc_failure_unit_MergeAnnotationNotType, "unit/MergeAnnotationNotType");
    tc_failure!(tc_failure_unit_MergeEmptyWithoutAnnotation, "unit/MergeEmptyWithoutAnnotation");
    tc_failure!(tc_failure_unit_MergeHandlerNotFunction, "unit/MergeHandlerNotFunction");
    tc_failure!(tc_failure_unit_MergeHandlerNotInUnion, "unit/MergeHandlerNotInUnion");
    tc_failure!(tc_failure_unit_MergeHandlerNotMatchAlternativeType, "unit/MergeHandlerNotMatchAlternativeType");
    tc_failure!(tc_failure_unit_MergeHandlersWithDifferentType, "unit/MergeHandlersWithDifferentType");
    tc_failure!(tc_failure_unit_MergeLhsNotRecord, "unit/MergeLhsNotRecord");
    tc_failure!(tc_failure_unit_MergeRhsNotUnion, "unit/MergeRhsNotUnion");
    tc_failure!(tc_failure_unit_MergeWithWrongAnnotation, "unit/MergeWithWrongAnnotation");
    tc_failure!(tc_failure_unit_OperatorAndNotBool, "unit/OperatorAndNotBool");
    tc_failure!(tc_failure_unit_OperatorEqualNotBool, "unit/OperatorEqualNotBool");
    tc_failure!(tc_failure_unit_OperatorListConcatenateLhsNotList, "unit/OperatorListConcatenateLhsNotList");
    tc_failure!(tc_failure_unit_OperatorListConcatenateListsNotMatch, "unit/OperatorListConcatenateListsNotMatch");
    tc_failure!(tc_failure_unit_OperatorListConcatenateNotListsButMatch, "unit/OperatorListConcatenateNotListsButMatch");
    tc_failure!(tc_failure_unit_OperatorListConcatenateRhsNotList, "unit/OperatorListConcatenateRhsNotList");
    tc_failure!(tc_failure_unit_OperatorNotEqualNotBool, "unit/OperatorNotEqualNotBool");
    tc_failure!(tc_failure_unit_OperatorOrNotBool, "unit/OperatorOrNotBool");
    tc_failure!(tc_failure_unit_OperatorPlusNotNatural, "unit/OperatorPlusNotNatural");
    tc_failure!(tc_failure_unit_OperatorTextConcatenateLhsNotText, "unit/OperatorTextConcatenateLhsNotText");
    tc_failure!(tc_failure_unit_OperatorTextConcatenateRhsNotText, "unit/OperatorTextConcatenateRhsNotText");
    tc_failure!(tc_failure_unit_OperatorTimesNotNatural, "unit/OperatorTimesNotNatural");
    tc_failure!(tc_failure_unit_RecordMixedKinds, "unit/RecordMixedKinds");
    tc_failure!(tc_failure_unit_RecordMixedKinds2, "unit/RecordMixedKinds2");
    tc_failure!(tc_failure_unit_RecordMixedKinds3, "unit/RecordMixedKinds3");
    tc_failure!(tc_failure_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    tc_failure!(tc_failure_unit_RecordProjectionNotPresent, "unit/RecordProjectionNotPresent");
    tc_failure!(tc_failure_unit_RecordProjectionNotRecord, "unit/RecordProjectionNotRecord");
    tc_failure!(tc_failure_unit_RecordSelectionEmpty, "unit/RecordSelectionEmpty");
    tc_failure!(tc_failure_unit_RecordSelectionNotPresent, "unit/RecordSelectionNotPresent");
    tc_failure!(tc_failure_unit_RecordSelectionNotRecord, "unit/RecordSelectionNotRecord");
    tc_failure!(tc_failure_unit_RecordSelectionTypeNotUnionType, "unit/RecordSelectionTypeNotUnionType");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds, "unit/RecordTypeMixedKinds");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds2, "unit/RecordTypeMixedKinds2");
    tc_failure!(tc_failure_unit_RecordTypeMixedKinds3, "unit/RecordTypeMixedKinds3");
    tc_failure!(tc_failure_unit_RecordTypeValueMember, "unit/RecordTypeValueMember");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeLhsNotRecord, "unit/RecursiveRecordMergeLhsNotRecord");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeMixedKinds, "unit/RecursiveRecordMergeMixedKinds");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeOverlapping, "unit/RecursiveRecordMergeOverlapping");
    tc_failure!(tc_failure_unit_RecursiveRecordMergeRhsNotRecord, "unit/RecursiveRecordMergeRhsNotRecord");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeLhsNotRecordType, "unit/RecursiveRecordTypeMergeLhsNotRecordType");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeOverlapping, "unit/RecursiveRecordTypeMergeOverlapping");
    tc_failure!(tc_failure_unit_RecursiveRecordTypeMergeRhsNotRecordType, "unit/RecursiveRecordTypeMergeRhsNotRecordType");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeLhsNotRecord, "unit/RightBiasedRecordMergeLhsNotRecord");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds, "unit/RightBiasedRecordMergeMixedKinds");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds2, "unit/RightBiasedRecordMergeMixedKinds2");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeMixedKinds3, "unit/RightBiasedRecordMergeMixedKinds3");
    tc_failure!(tc_failure_unit_RightBiasedRecordMergeRhsNotRecord, "unit/RightBiasedRecordMergeRhsNotRecord");
    tc_failure!(tc_failure_unit_SomeNotType, "unit/SomeNotType");
    tc_failure!(tc_failure_unit_Sort, "unit/Sort");
    tc_failure!(tc_failure_unit_TextLiteralInterpolateNotText, "unit/TextLiteralInterpolateNotText");
    tc_failure!(tc_failure_unit_TypeAnnotationWrong, "unit/TypeAnnotationWrong");
    tc_failure!(tc_failure_unit_UnionConstructorFieldNotPresent, "unit/UnionConstructorFieldNotPresent");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds, "unit/UnionTypeMixedKinds");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds2, "unit/UnionTypeMixedKinds2");
    tc_failure!(tc_failure_unit_UnionTypeMixedKinds3, "unit/UnionTypeMixedKinds3");
    tc_failure!(tc_failure_unit_UnionTypeNotType, "unit/UnionTypeNotType");
    tc_failure!(tc_failure_unit_VariableFree, "unit/VariableFree");

    ti_success!(ti_success_simple_alternativesAreTypes, "simple/alternativesAreTypes");
    ti_success!(ti_success_simple_kindParameter, "simple/kindParameter");
    ti_success!(ti_success_unit_Bool, "unit/Bool");
    ti_success!(ti_success_unit_Double, "unit/Double");
    ti_success!(ti_success_unit_DoubleLiteral, "unit/DoubleLiteral");
    ti_success!(ti_success_unit_DoubleShow, "unit/DoubleShow");
    ti_success!(ti_success_unit_False, "unit/False");
    ti_success!(ti_success_unit_Function, "unit/Function");
    ti_success!(ti_success_unit_FunctionApplication, "unit/FunctionApplication");
    ti_success!(ti_success_unit_FunctionNamedArg, "unit/FunctionNamedArg");
    ti_success!(ti_success_unit_FunctionTypeKindKind, "unit/FunctionTypeKindKind");
    ti_success!(ti_success_unit_FunctionTypeKindTerm, "unit/FunctionTypeKindTerm");
    ti_success!(ti_success_unit_FunctionTypeKindType, "unit/FunctionTypeKindType");
    ti_success!(ti_success_unit_FunctionTypeTermTerm, "unit/FunctionTypeTermTerm");
    ti_success!(ti_success_unit_FunctionTypeTypeTerm, "unit/FunctionTypeTypeTerm");
    ti_success!(ti_success_unit_FunctionTypeTypeType, "unit/FunctionTypeTypeType");
    ti_success!(ti_success_unit_FunctionTypeUsingArgument, "unit/FunctionTypeUsingArgument");
    ti_success!(ti_success_unit_If, "unit/If");
    ti_success!(ti_success_unit_IfNormalizeArguments, "unit/IfNormalizeArguments");
    ti_success!(ti_success_unit_Integer, "unit/Integer");
    ti_success!(ti_success_unit_IntegerLiteral, "unit/IntegerLiteral");
    ti_success!(ti_success_unit_IntegerShow, "unit/IntegerShow");
    ti_success!(ti_success_unit_IntegerToDouble, "unit/IntegerToDouble");
    ti_success!(ti_success_unit_Kind, "unit/Kind");
    ti_success!(ti_success_unit_Let, "unit/Let");
    ti_success!(ti_success_unit_LetNestedTypeSynonym, "unit/LetNestedTypeSynonym");
    ti_success!(ti_success_unit_LetTypeSynonym, "unit/LetTypeSynonym");
    ti_success!(ti_success_unit_LetWithAnnotation, "unit/LetWithAnnotation");
    ti_success!(ti_success_unit_List, "unit/List");
    ti_success!(ti_success_unit_ListBuild, "unit/ListBuild");
    ti_success!(ti_success_unit_ListFold, "unit/ListFold");
    ti_success!(ti_success_unit_ListHead, "unit/ListHead");
    ti_success!(ti_success_unit_ListIndexed, "unit/ListIndexed");
    ti_success!(ti_success_unit_ListLast, "unit/ListLast");
    ti_success!(ti_success_unit_ListLength, "unit/ListLength");
    ti_success!(ti_success_unit_ListLiteralEmpty, "unit/ListLiteralEmpty");
    ti_success!(ti_success_unit_ListLiteralNormalizeArguments, "unit/ListLiteralNormalizeArguments");
    ti_success!(ti_success_unit_ListLiteralOne, "unit/ListLiteralOne");
    ti_success!(ti_success_unit_ListReverse, "unit/ListReverse");
    ti_success!(ti_success_unit_MergeEmptyUnion, "unit/MergeEmptyUnion");
    ti_success!(ti_success_unit_MergeOne, "unit/MergeOne");
    ti_success!(ti_success_unit_MergeOneWithAnnotation, "unit/MergeOneWithAnnotation");
    ti_success!(ti_success_unit_Natural, "unit/Natural");
    ti_success!(ti_success_unit_NaturalBuild, "unit/NaturalBuild");
    ti_success!(ti_success_unit_NaturalEven, "unit/NaturalEven");
    ti_success!(ti_success_unit_NaturalFold, "unit/NaturalFold");
    ti_success!(ti_success_unit_NaturalIsZero, "unit/NaturalIsZero");
    ti_success!(ti_success_unit_NaturalLiteral, "unit/NaturalLiteral");
    ti_success!(ti_success_unit_NaturalOdd, "unit/NaturalOdd");
    ti_success!(ti_success_unit_NaturalShow, "unit/NaturalShow");
    ti_success!(ti_success_unit_NaturalToInteger, "unit/NaturalToInteger");
    ti_success!(ti_success_unit_None, "unit/None");
    ti_success!(ti_success_unit_OldOptionalNone, "unit/OldOptionalNone");
    ti_success!(ti_success_unit_OldOptionalTrue, "unit/OldOptionalTrue");
    ti_success!(ti_success_unit_OperatorAnd, "unit/OperatorAnd");
    ti_success!(ti_success_unit_OperatorAndNormalizeArguments, "unit/OperatorAndNormalizeArguments");
    ti_success!(ti_success_unit_OperatorEqual, "unit/OperatorEqual");
    ti_success!(ti_success_unit_OperatorEqualNormalizeArguments, "unit/OperatorEqualNormalizeArguments");
    ti_success!(ti_success_unit_OperatorListConcatenate, "unit/OperatorListConcatenate");
    ti_success!(ti_success_unit_OperatorListConcatenateNormalizeArguments, "unit/OperatorListConcatenateNormalizeArguments");
    ti_success!(ti_success_unit_OperatorNotEqual, "unit/OperatorNotEqual");
    ti_success!(ti_success_unit_OperatorNotEqualNormalizeArguments, "unit/OperatorNotEqualNormalizeArguments");
    ti_success!(ti_success_unit_OperatorOr, "unit/OperatorOr");
    ti_success!(ti_success_unit_OperatorOrNormalizeArguments, "unit/OperatorOrNormalizeArguments");
    ti_success!(ti_success_unit_OperatorPlus, "unit/OperatorPlus");
    ti_success!(ti_success_unit_OperatorPlusNormalizeArguments, "unit/OperatorPlusNormalizeArguments");
    ti_success!(ti_success_unit_OperatorTextConcatenate, "unit/OperatorTextConcatenate");
    ti_success!(ti_success_unit_OperatorTextConcatenateNormalizeArguments, "unit/OperatorTextConcatenateNormalizeArguments");
    ti_success!(ti_success_unit_OperatorTimes, "unit/OperatorTimes");
    ti_success!(ti_success_unit_OperatorTimesNormalizeArguments, "unit/OperatorTimesNormalizeArguments");
    ti_success!(ti_success_unit_Optional, "unit/Optional");
    ti_success!(ti_success_unit_OptionalBuild, "unit/OptionalBuild");
    ti_success!(ti_success_unit_OptionalFold, "unit/OptionalFold");
    ti_success!(ti_success_unit_RecordEmpty, "unit/RecordEmpty");
    ti_success!(ti_success_unit_RecordNestedKind, "unit/RecordNestedKind");
    ti_success!(ti_success_unit_RecordNestedKindLike, "unit/RecordNestedKindLike");
    ti_success!(ti_success_unit_RecordNestedType, "unit/RecordNestedType");
    ti_success!(ti_success_unit_RecordNestedTypeLike, "unit/RecordNestedTypeLike");
    ti_success!(ti_success_unit_RecordOneKind, "unit/RecordOneKind");
    ti_success!(ti_success_unit_RecordOneType, "unit/RecordOneType");
    ti_success!(ti_success_unit_RecordOneValue, "unit/RecordOneValue");
    ti_success!(ti_success_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    ti_success!(ti_success_unit_RecordProjectionKind, "unit/RecordProjectionKind");
    ti_success!(ti_success_unit_RecordProjectionType, "unit/RecordProjectionType");
    ti_success!(ti_success_unit_RecordProjectionValue, "unit/RecordProjectionValue");
    ti_success!(ti_success_unit_RecordSelectionKind, "unit/RecordSelectionKind");
    ti_success!(ti_success_unit_RecordSelectionType, "unit/RecordSelectionType");
    ti_success!(ti_success_unit_RecordSelectionValue, "unit/RecordSelectionValue");
    ti_success!(ti_success_unit_RecordType, "unit/RecordType");
    ti_success!(ti_success_unit_RecordTypeEmpty, "unit/RecordTypeEmpty");
    ti_success!(ti_success_unit_RecordTypeKind, "unit/RecordTypeKind");
    ti_success!(ti_success_unit_RecordTypeKindLike, "unit/RecordTypeKindLike");
    ti_success!(ti_success_unit_RecordTypeNestedKind, "unit/RecordTypeNestedKind");
    ti_success!(ti_success_unit_RecordTypeNestedKindLike, "unit/RecordTypeNestedKindLike");
    ti_success!(ti_success_unit_RecordTypeType, "unit/RecordTypeType");
    ti_success!(ti_success_unit_RecursiveRecordMergeLhsEmpty, "unit/RecursiveRecordMergeLhsEmpty");
    ti_success!(ti_success_unit_RecursiveRecordMergeRecursively, "unit/RecursiveRecordMergeRecursively");
    ti_success!(ti_success_unit_RecursiveRecordMergeRecursivelyKinds, "unit/RecursiveRecordMergeRecursivelyKinds");
    ti_success!(ti_success_unit_RecursiveRecordMergeRecursivelyTypes, "unit/RecursiveRecordMergeRecursivelyTypes");
    ti_success!(ti_success_unit_RecursiveRecordMergeRhsEmpty, "unit/RecursiveRecordMergeRhsEmpty");
    ti_success!(ti_success_unit_RecursiveRecordMergeTwo, "unit/RecursiveRecordMergeTwo");
    ti_success!(ti_success_unit_RecursiveRecordMergeTwoKinds, "unit/RecursiveRecordMergeTwoKinds");
    ti_success!(ti_success_unit_RecursiveRecordMergeTwoTypes, "unit/RecursiveRecordMergeTwoTypes");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRecursively, "unit/RecursiveRecordTypeMergeRecursively");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRecursivelyKinds, "unit/RecursiveRecordTypeMergeRecursivelyKinds");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRecursivelyTypes, "unit/RecursiveRecordTypeMergeRecursivelyTypes");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeRhsEmpty, "unit/RecursiveRecordTypeMergeRhsEmpty");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwo, "unit/RecursiveRecordTypeMergeTwo");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwoKinds, "unit/RecursiveRecordTypeMergeTwoKinds");
    // ti_success!(ti_success_unit_RecursiveRecordTypeMergeTwoTypes, "unit/RecursiveRecordTypeMergeTwoTypes");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeRhsEmpty, "unit/RightBiasedRecordMergeRhsEmpty");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwo, "unit/RightBiasedRecordMergeTwo");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoDifferent, "unit/RightBiasedRecordMergeTwoDifferent");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoKinds, "unit/RightBiasedRecordMergeTwoKinds");
    // ti_success!(ti_success_unit_RightBiasedRecordMergeTwoTypes, "unit/RightBiasedRecordMergeTwoTypes");
    ti_success!(ti_success_unit_SomeTrue, "unit/SomeTrue");
    ti_success!(ti_success_unit_Text, "unit/Text");
    ti_success!(ti_success_unit_TextLiteral, "unit/TextLiteral");
    ti_success!(ti_success_unit_TextLiteralNormalizeArguments, "unit/TextLiteralNormalizeArguments");
    ti_success!(ti_success_unit_TextLiteralWithInterpolation, "unit/TextLiteralWithInterpolation");
    ti_success!(ti_success_unit_TextShow, "unit/TextShow");
    ti_success!(ti_success_unit_True, "unit/True");
    ti_success!(ti_success_unit_Type, "unit/Type");
    ti_success!(ti_success_unit_TypeAnnotation, "unit/TypeAnnotation");
    ti_success!(ti_success_unit_TypeAnnotationSort, "unit/TypeAnnotationSort");
    ti_success!(ti_success_unit_UnionConstructorEmptyField, "unit/UnionConstructorEmptyField");
    ti_success!(ti_success_unit_UnionConstructorField, "unit/UnionConstructorField");
    ti_success!(ti_success_unit_UnionOne, "unit/UnionOne");
    ti_success!(ti_success_unit_UnionTypeEmpty, "unit/UnionTypeEmpty");
    ti_success!(ti_success_unit_UnionTypeKind, "unit/UnionTypeKind");
    ti_success!(ti_success_unit_UnionTypeOne, "unit/UnionTypeOne");
    ti_success!(ti_success_unit_UnionTypeType, "unit/UnionTypeType");
}
