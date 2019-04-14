#![allow(non_snake_case)]
use crate::expr::*;
use dhall_core::*;
use dhall_generator::dhall_expr;
use std::fmt;

impl<'a> Typed<'a> {
    pub fn normalize(self) -> Normalized<'a> {
        Normalized(normalize(self.0), self.1, self.2)
    }
    /// Pretends this expression is normalized. Use with care.
    #[allow(dead_code)]
    pub fn skip_normalize(self) -> Normalized<'a> {
        Normalized(
            self.0.unroll().squash_embed(|e| e.0.clone()),
            self.1,
            self.2,
        )
    }
}

fn apply_builtin<S, A>(b: Builtin, args: &[Expr<S, A>]) -> WhatNext<S, A>
where
    S: fmt::Debug + Clone,
    A: fmt::Debug + Clone,
{
    use dhall_core::Builtin::*;
    use dhall_core::ExprF::*;
    use WhatNext::*;
    let (ret, rest) = match (b, args) {
        (OptionalNone, [t, rest..]) => (rc(EmptyOptionalLit(t.roll())), rest),
        (NaturalIsZero, [NaturalLit(n), rest..]) => {
            (rc(BoolLit(*n == 0)), rest)
        }
        (NaturalEven, [NaturalLit(n), rest..]) => {
            (rc(BoolLit(*n % 2 == 0)), rest)
        }
        (NaturalOdd, [NaturalLit(n), rest..]) => {
            (rc(BoolLit(*n % 2 != 0)), rest)
        }
        (NaturalToInteger, [NaturalLit(n), rest..]) => {
            (rc(IntegerLit(*n as isize)), rest)
        }
        (NaturalShow, [NaturalLit(n), rest..]) => {
            (rc(TextLit(n.to_string().into())), rest)
        }
        (ListLength, [_, EmptyListLit(_), rest..]) => (rc(NaturalLit(0)), rest),
        (ListLength, [_, NEListLit(ys), rest..]) => {
            (rc(NaturalLit(ys.len())), rest)
        }
        (ListHead, [_, EmptyListLit(t), rest..]) => {
            (rc(EmptyOptionalLit(t.clone())), rest)
        }
        (ListHead, [_, NEListLit(ys), rest..]) => {
            (rc(NEOptionalLit(ys.first().unwrap().clone())), rest)
        }
        (ListLast, [_, EmptyListLit(t), rest..]) => {
            (rc(EmptyOptionalLit(t.clone())), rest)
        }
        (ListLast, [_, NEListLit(ys), rest..]) => {
            (rc(NEOptionalLit(ys.last().unwrap().clone())), rest)
        }
        (ListReverse, [_, EmptyListLit(t), rest..]) => {
            (rc(EmptyListLit(t.clone())), rest)
        }
        (ListReverse, [_, NEListLit(ys), rest..]) => {
            let ys = ys.iter().rev().cloned().collect();
            (rc(NEListLit(ys)), rest)
        }
        (ListIndexed, [_, EmptyListLit(t), rest..]) => (
            dhall_expr!([] : List ({ index : Natural, value : t })),
            rest,
        ),
        (ListIndexed, [_, NEListLit(xs), rest..]) => {
            let xs = xs
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, e)| {
                    let i = rc(NaturalLit(i));
                    dhall_expr!({ index = i, value = e })
                })
                .collect();
            (rc(NEListLit(xs)), rest)
        }
        (ListBuild, [a0, g, rest..]) => {
            'ret: {
                if let App(f2, args2) = g {
                    if let (Builtin(ListFold), [_, x, rest_inner..]) =
                        (f2.as_ref(), args2.as_slice())
                    {
                        // fold/build fusion
                        break 'ret (
                            rc(App(x.clone(), rest_inner.to_vec())),
                            rest,
                        );
                    }
                };
                let a0 = a0.roll();
                let a1 = shift(1, &V("x".into(), 0), &a0);
                let g = g.roll();
                break 'ret (
                    dhall_expr!(
                        g
                        (List a0)
                        (λ(x : a0) -> λ(xs : List a1) -> [ x ] # xs)
                        ([] : List a0)
                    ),
                    rest,
                );
            }
        }
        (OptionalBuild, [a0, g, rest..]) => {
            'ret: {
                if let App(f2, args2) = g {
                    if let (Builtin(OptionalFold), [_, x, rest_inner..]) =
                        (f2.as_ref(), args2.as_slice())
                    {
                        // fold/build fusion
                        break 'ret (
                            rc(App(x.clone(), rest_inner.to_vec())),
                            rest,
                        );
                    }
                };
                let a0 = a0.roll();
                let g = g.roll();
                break 'ret (
                    dhall_expr!(
                        g
                        (Optional a0)
                        (λ(x: a0) -> Some x)
                        (None a0)
                    ),
                    rest,
                );
            }
        }
        (ListFold, [_, EmptyListLit(_), _, _, nil, rest..]) => {
            (nil.roll(), rest)
        }
        (ListFold, [_, NEListLit(xs), _, cons, nil, rest..]) => (
            xs.iter().rev().fold(nil.roll(), |acc, x| {
                let x = x.clone();
                let acc = acc.clone();
                let cons = cons.roll();
                dhall_expr!(cons x acc)
            }),
            rest,
        ),
        // // fold/build fusion
        // (ListFold, [_, App(box Builtin(ListBuild), [_, x, rest..]), rest..]) => {
        //     normalize_ref(&App(bx(x.clone()), rest.to_vec()))
        // }
        (OptionalFold, [_, NEOptionalLit(x), _, just, _, rest..]) => {
            let x = x.clone();
            let just = just.roll();
            (dhall_expr!(just x), rest)
        }
        (OptionalFold, [_, EmptyOptionalLit(_), _, _, nothing, rest..]) => {
            (nothing.roll(), rest)
        }
        // // fold/build fusion
        // (OptionalFold, [_, App(box Builtin(OptionalBuild), [_, x, rest..]), rest..]) => {
        //     normalize_ref(&App(bx(x.clone()), rest.to_vec()))
        // }
        (NaturalBuild, [g, rest..]) => {
            'ret: {
                if let App(f2, args2) = g {
                    if let (Builtin(NaturalFold), [x, rest_inner..]) =
                        (f2.as_ref(), args2.as_slice())
                    {
                        // fold/build fusion
                        break 'ret (
                            rc(App(x.clone(), rest_inner.to_vec())),
                            rest,
                        );
                    }
                };
                let g = g.roll();
                break 'ret (
                    dhall_expr!(g Natural (λ(x : Natural) -> x + 1) 0),
                    rest,
                );
            }
        }
        (NaturalFold, [NaturalLit(0), _, _, zero, rest..]) => {
            (zero.roll(), rest)
        }
        (NaturalFold, [NaturalLit(n), t, succ, zero, rest..]) => {
            let fold = rc(Builtin(NaturalFold));
            let n = rc(NaturalLit(n - 1));
            let t = t.roll();
            let succ = succ.roll();
            let zero = zero.roll();
            (dhall_expr!(succ (fold n t succ zero)), rest)
        }
        // (NaturalFold, Some(App(f2, args2)), _) => {
        //     match (f2.as_ref(), args2.as_slice()) {
        //         // fold/build fusion
        //         (Builtin(NaturalBuild), [x, rest..]) => {
        //             rc(App(x.clone(), rest.to_vec()))
        //         }
        //         _ => return rc(App(f, args)),
        //     }
        // }
        (IntegerShow, [IntegerLit(n), rest..]) => {
            let plus = if n < &0 { "" } else { "+" };
            (rc(TextLit((plus.to_owned() + &n.to_string()).into())), rest)
        }
        _ => return DoneAsIs,
    };
    // Put the remaining arguments back and eval again. In most cases
    // ret will not be of a form that can be applied, so this won't go very deep.
    // In lots of cases, there are no remaining args so this cann will just return ret.
    let rest: Vec<SubExpr<S, A>> = rest.iter().map(ExprF::roll).collect();
    Continue(ExprF::App(ret, rest))
}

// Small enum to help with being DRY
enum WhatNext<'a, S, A> {
    // Recurse on this expression
    Continue(Expr<S, A>),
    ContinueSub(SubExpr<S, A>),
    // The following expression is the normal form
    Done(Expr<S, A>),
    DoneRef(&'a Expr<S, A>),
    DoneRefSub(&'a SubExpr<S, A>),
    // The current expression is already in normal form
    DoneAsIs,
}

fn normalize_ref(expr: &Expr<X, Normalized<'static>>) -> Expr<X, X> {
    use dhall_core::BinOp::*;
    use dhall_core::ExprF::*;
    // Recursively normalize all subexpressions
    let expr: ExprF<Expr<X, X>, Label, X, Normalized<'static>> =
        expr.map_ref_simple(|e| normalize_ref(e.as_ref()));

    use WhatNext::*;
    let what_next = match &expr {
        Let(f, _, r, b) => {
            let vf0 = &V(f.clone(), 0);
            // TODO: use a context
            ContinueSub(subst_shift(vf0, &r.roll(), &b.roll()))
        }
        Annot(x, _) => DoneRef(x),
        Note(_, e) => DoneRef(e),
        App(f, args) if args.is_empty() => DoneRef(f),
        App(App(f, args1), args2) => Continue(App(
            f.clone(),
            args1
                .iter()
                .cloned()
                .chain(args2.iter().map(ExprF::roll))
                .collect(),
        )),
        App(Builtin(b), args) => apply_builtin(*b, args),
        App(Lam(x, _, b), args) => {
            let mut iter = args.iter();
            // We know args is nonempty
            let a = iter.next().unwrap();
            // Beta reduce
            let vx0 = &V(x.clone(), 0);
            let b2 = subst_shift(vx0, &a.roll(), &b);
            Continue(App(b2, iter.map(ExprF::roll).collect()))
        }
        BoolIf(BoolLit(true), t, _) => DoneRef(t),
        BoolIf(BoolLit(false), _, f) => DoneRef(f),
        // TODO: interpolation
        // TextLit(t) =>
        BinOp(BoolAnd, BoolLit(x), BoolLit(y)) => Done(BoolLit(*x && *y)),
        BinOp(BoolOr, BoolLit(x), BoolLit(y)) => Done(BoolLit(*x || *y)),
        BinOp(BoolEQ, BoolLit(x), BoolLit(y)) => Done(BoolLit(x == y)),
        BinOp(BoolNE, BoolLit(x), BoolLit(y)) => Done(BoolLit(x != y)),
        BinOp(NaturalPlus, NaturalLit(x), NaturalLit(y)) => {
            Done(NaturalLit(x + y))
        }
        BinOp(NaturalTimes, NaturalLit(x), NaturalLit(y)) => {
            Done(NaturalLit(x * y))
        }
        BinOp(TextAppend, TextLit(x), TextLit(y)) => Done(TextLit(x + y)),
        BinOp(ListAppend, EmptyListLit(_), y) => DoneRef(y),
        BinOp(ListAppend, x, EmptyListLit(_)) => DoneRef(x),
        BinOp(ListAppend, NEListLit(xs), NEListLit(ys)) => {
            let xs = xs.iter().cloned();
            let ys = ys.iter().cloned();
            Done(NEListLit(xs.chain(ys).collect()))
        }
        Merge(RecordLit(handlers), UnionLit(k, v, _), _) => {
            match handlers.get(&k) {
                Some(h) => Continue(App(h.clone(), vec![v.clone()])),
                None => DoneAsIs,
            }
        }
        Field(RecordLit(kvs), l) => match kvs.get(&l) {
            Some(r) => DoneRefSub(r),
            None => DoneAsIs,
        },
        Projection(_, ls) if ls.is_empty() => {
            Done(RecordLit(std::collections::BTreeMap::new()))
        }
        Projection(RecordLit(kvs), ls) => Done(RecordLit(
            ls.iter()
                .filter_map(|l| kvs.get(l).map(|x| (l.clone(), x.clone())))
                .collect(),
        )),
        Embed(e) => DoneRefSub(&e.0),
        _ => DoneAsIs,
    };

    match what_next {
        Continue(e) => normalize_ref(&e.absurd_rec()),
        ContinueSub(e) => normalize_ref(e.absurd().as_ref()),
        Done(e) => e,
        DoneRef(e) => e.clone(),
        DoneRefSub(e) => e.unroll(),
        DoneAsIs => match expr.map_ref_simple(ExprF::roll) {
            e => e.map_ref(
                |e| e.clone(),
                |_, e| e.clone(),
                X::clone,
                |_| unreachable!(),
                Label::clone,
            ),
        },
    }
}

/// Reduce an expression to its normal form, performing beta reduction
///
/// `normalize` does not type-check the expression.  You may want to type-check
/// expressions before normalizing them since normalization can convert an
/// ill-typed expression into a well-typed expression.
///
/// However, `normalize` will not fail if the expression is ill-typed and will
/// leave ill-typed sub-expressions unevaluated.
///
fn normalize(e: SubExpr<X, Normalized<'static>>) -> SubExpr<X, X> {
    normalize_ref(e.as_ref()).roll()
}

#[cfg(test)]
mod spec_tests {
    #![rustfmt::skip]

    macro_rules! norm {
        ($name:ident, $path:expr) => {
            make_spec_test!(Normalization, Success, $name, $path);
        };
    }

    norm!(success_haskell_tutorial_access_0, "haskell-tutorial/access/0");
    // norm!(success_haskell_tutorial_access_1, "haskell-tutorial/access/1");
    // norm!(success_haskell_tutorial_combineTypes_0, "haskell-tutorial/combineTypes/0");
    // norm!(success_haskell_tutorial_combineTypes_1, "haskell-tutorial/combineTypes/1");
    // norm!(success_haskell_tutorial_prefer_0, "haskell-tutorial/prefer/0");
    norm!(success_haskell_tutorial_projection_0, "haskell-tutorial/projection/0");


    norm!(success_prelude_Bool_and_0, "prelude/Bool/and/0");
    norm!(success_prelude_Bool_and_1, "prelude/Bool/and/1");
    norm!(success_prelude_Bool_build_0, "prelude/Bool/build/0");
    norm!(success_prelude_Bool_build_1, "prelude/Bool/build/1");
    norm!(success_prelude_Bool_even_0, "prelude/Bool/even/0");
    norm!(success_prelude_Bool_even_1, "prelude/Bool/even/1");
    norm!(success_prelude_Bool_even_2, "prelude/Bool/even/2");
    norm!(success_prelude_Bool_even_3, "prelude/Bool/even/3");
    norm!(success_prelude_Bool_fold_0, "prelude/Bool/fold/0");
    norm!(success_prelude_Bool_fold_1, "prelude/Bool/fold/1");
    norm!(success_prelude_Bool_not_0, "prelude/Bool/not/0");
    norm!(success_prelude_Bool_not_1, "prelude/Bool/not/1");
    norm!(success_prelude_Bool_odd_0, "prelude/Bool/odd/0");
    norm!(success_prelude_Bool_odd_1, "prelude/Bool/odd/1");
    norm!(success_prelude_Bool_odd_2, "prelude/Bool/odd/2");
    norm!(success_prelude_Bool_odd_3, "prelude/Bool/odd/3");
    norm!(success_prelude_Bool_or_0, "prelude/Bool/or/0");
    norm!(success_prelude_Bool_or_1, "prelude/Bool/or/1");
    norm!(success_prelude_Bool_show_0, "prelude/Bool/show/0");
    norm!(success_prelude_Bool_show_1, "prelude/Bool/show/1");
    // norm!(success_prelude_Double_show_0, "prelude/Double/show/0");
    // norm!(success_prelude_Double_show_1, "prelude/Double/show/1");
    // norm!(success_prelude_Integer_show_0, "prelude/Integer/show/0");
    // norm!(success_prelude_Integer_show_1, "prelude/Integer/show/1");
    // norm!(success_prelude_Integer_toDouble_0, "prelude/Integer/toDouble/0");
    // norm!(success_prelude_Integer_toDouble_1, "prelude/Integer/toDouble/1");
    norm!(success_prelude_List_all_0, "prelude/List/all/0");
    norm!(success_prelude_List_all_1, "prelude/List/all/1");
    norm!(success_prelude_List_any_0, "prelude/List/any/0");
    norm!(success_prelude_List_any_1, "prelude/List/any/1");
    norm!(success_prelude_List_build_0, "prelude/List/build/0");
    norm!(success_prelude_List_build_1, "prelude/List/build/1");
    norm!(success_prelude_List_concat_0, "prelude/List/concat/0");
    norm!(success_prelude_List_concat_1, "prelude/List/concat/1");
    norm!(success_prelude_List_concatMap_0, "prelude/List/concatMap/0");
    norm!(success_prelude_List_concatMap_1, "prelude/List/concatMap/1");
    norm!(success_prelude_List_filter_0, "prelude/List/filter/0");
    norm!(success_prelude_List_filter_1, "prelude/List/filter/1");
    norm!(success_prelude_List_fold_0, "prelude/List/fold/0");
    norm!(success_prelude_List_fold_1, "prelude/List/fold/1");
    norm!(success_prelude_List_fold_2, "prelude/List/fold/2");
    norm!(success_prelude_List_generate_0, "prelude/List/generate/0");
    norm!(success_prelude_List_generate_1, "prelude/List/generate/1");
    norm!(success_prelude_List_head_0, "prelude/List/head/0");
    norm!(success_prelude_List_head_1, "prelude/List/head/1");
    norm!(success_prelude_List_indexed_0, "prelude/List/indexed/0");
    norm!(success_prelude_List_indexed_1, "prelude/List/indexed/1");
    norm!(success_prelude_List_iterate_0, "prelude/List/iterate/0");
    norm!(success_prelude_List_iterate_1, "prelude/List/iterate/1");
    norm!(success_prelude_List_last_0, "prelude/List/last/0");
    norm!(success_prelude_List_last_1, "prelude/List/last/1");
    norm!(success_prelude_List_length_0, "prelude/List/length/0");
    norm!(success_prelude_List_length_1, "prelude/List/length/1");
    norm!(success_prelude_List_map_0, "prelude/List/map/0");
    norm!(success_prelude_List_map_1, "prelude/List/map/1");
    norm!(success_prelude_List_null_0, "prelude/List/null/0");
    norm!(success_prelude_List_null_1, "prelude/List/null/1");
    norm!(success_prelude_List_replicate_0, "prelude/List/replicate/0");
    norm!(success_prelude_List_replicate_1, "prelude/List/replicate/1");
    norm!(success_prelude_List_reverse_0, "prelude/List/reverse/0");
    norm!(success_prelude_List_reverse_1, "prelude/List/reverse/1");
    norm!(success_prelude_List_shifted_0, "prelude/List/shifted/0");
    norm!(success_prelude_List_shifted_1, "prelude/List/shifted/1");
    norm!(success_prelude_List_unzip_0, "prelude/List/unzip/0");
    norm!(success_prelude_List_unzip_1, "prelude/List/unzip/1");
    norm!(success_prelude_Natural_build_0, "prelude/Natural/build/0");
    norm!(success_prelude_Natural_build_1, "prelude/Natural/build/1");
    norm!(success_prelude_Natural_enumerate_0, "prelude/Natural/enumerate/0");
    norm!(success_prelude_Natural_enumerate_1, "prelude/Natural/enumerate/1");
    norm!(success_prelude_Natural_even_0, "prelude/Natural/even/0");
    norm!(success_prelude_Natural_even_1, "prelude/Natural/even/1");
    norm!(success_prelude_Natural_fold_0, "prelude/Natural/fold/0");
    norm!(success_prelude_Natural_fold_1, "prelude/Natural/fold/1");
    norm!(success_prelude_Natural_fold_2, "prelude/Natural/fold/2");
    norm!(success_prelude_Natural_isZero_0, "prelude/Natural/isZero/0");
    norm!(success_prelude_Natural_isZero_1, "prelude/Natural/isZero/1");
    norm!(success_prelude_Natural_odd_0, "prelude/Natural/odd/0");
    norm!(success_prelude_Natural_odd_1, "prelude/Natural/odd/1");
    norm!(success_prelude_Natural_product_0, "prelude/Natural/product/0");
    norm!(success_prelude_Natural_product_1, "prelude/Natural/product/1");
    // norm!(success_prelude_Natural_show_0, "prelude/Natural/show/0");
    // norm!(success_prelude_Natural_show_1, "prelude/Natural/show/1");
    norm!(success_prelude_Natural_sum_0, "prelude/Natural/sum/0");
    norm!(success_prelude_Natural_sum_1, "prelude/Natural/sum/1");
    // norm!(success_prelude_Natural_toDouble_0, "prelude/Natural/toDouble/0");
    // norm!(success_prelude_Natural_toDouble_1, "prelude/Natural/toDouble/1");
    // norm!(success_prelude_Natural_toInteger_0, "prelude/Natural/toInteger/0");
    // norm!(success_prelude_Natural_toInteger_1, "prelude/Natural/toInteger/1");
    norm!(success_prelude_Optional_all_0, "prelude/Optional/all/0");
    norm!(success_prelude_Optional_all_1, "prelude/Optional/all/1");
    norm!(success_prelude_Optional_any_0, "prelude/Optional/any/0");
    norm!(success_prelude_Optional_any_1, "prelude/Optional/any/1");
    // norm!(success_prelude_Optional_build_0, "prelude/Optional/build/0");
    // norm!(success_prelude_Optional_build_1, "prelude/Optional/build/1");
    norm!(success_prelude_Optional_concat_0, "prelude/Optional/concat/0");
    norm!(success_prelude_Optional_concat_1, "prelude/Optional/concat/1");
    norm!(success_prelude_Optional_concat_2, "prelude/Optional/concat/2");
    // norm!(success_prelude_Optional_filter_0, "prelude/Optional/filter/0");
    // norm!(success_prelude_Optional_filter_1, "prelude/Optional/filter/1");
    norm!(success_prelude_Optional_fold_0, "prelude/Optional/fold/0");
    norm!(success_prelude_Optional_fold_1, "prelude/Optional/fold/1");
    norm!(success_prelude_Optional_head_0, "prelude/Optional/head/0");
    norm!(success_prelude_Optional_head_1, "prelude/Optional/head/1");
    norm!(success_prelude_Optional_head_2, "prelude/Optional/head/2");
    norm!(success_prelude_Optional_last_0, "prelude/Optional/last/0");
    norm!(success_prelude_Optional_last_1, "prelude/Optional/last/1");
    norm!(success_prelude_Optional_last_2, "prelude/Optional/last/2");
    norm!(success_prelude_Optional_length_0, "prelude/Optional/length/0");
    norm!(success_prelude_Optional_length_1, "prelude/Optional/length/1");
    norm!(success_prelude_Optional_map_0, "prelude/Optional/map/0");
    norm!(success_prelude_Optional_map_1, "prelude/Optional/map/1");
    norm!(success_prelude_Optional_null_0, "prelude/Optional/null/0");
    norm!(success_prelude_Optional_null_1, "prelude/Optional/null/1");
    norm!(success_prelude_Optional_toList_0, "prelude/Optional/toList/0");
    norm!(success_prelude_Optional_toList_1, "prelude/Optional/toList/1");
    norm!(success_prelude_Optional_unzip_0, "prelude/Optional/unzip/0");
    norm!(success_prelude_Optional_unzip_1, "prelude/Optional/unzip/1");
    norm!(success_prelude_Text_concat_0, "prelude/Text/concat/0");
    norm!(success_prelude_Text_concat_1, "prelude/Text/concat/1");
    // norm!(success_prelude_Text_concatMap_0, "prelude/Text/concatMap/0");
    norm!(success_prelude_Text_concatMap_1, "prelude/Text/concatMap/1");
    // norm!(success_prelude_Text_concatMapSep_0, "prelude/Text/concatMapSep/0");
    // norm!(success_prelude_Text_concatMapSep_1, "prelude/Text/concatMapSep/1");
    // norm!(success_prelude_Text_concatSep_0, "prelude/Text/concatSep/0");
    // norm!(success_prelude_Text_concatSep_1, "prelude/Text/concatSep/1");
    // norm!(success_prelude_Text_show_0, "prelude/Text/show/0");
    // norm!(success_prelude_Text_show_1, "prelude/Text/show/1");



    // norm!(success_remoteSystems, "remoteSystems");
    // norm!(success_simple_doubleShow, "simple/doubleShow");
    // norm!(success_simple_integerShow, "simple/integerShow");
    // norm!(success_simple_integerToDouble, "simple/integerToDouble");
    // norm!(success_simple_letlet, "simple/letlet");
    norm!(success_simple_listBuild, "simple/listBuild");
    norm!(success_simple_multiLine, "simple/multiLine");
    norm!(success_simple_naturalBuild, "simple/naturalBuild");
    norm!(success_simple_naturalPlus, "simple/naturalPlus");
    norm!(success_simple_naturalShow, "simple/naturalShow");
    norm!(success_simple_naturalToInteger, "simple/naturalToInteger");
    norm!(success_simple_optionalBuild, "simple/optionalBuild");
    norm!(success_simple_optionalBuildFold, "simple/optionalBuildFold");
    norm!(success_simple_optionalFold, "simple/optionalFold");
    // norm!(success_simple_sortOperator, "simple/sortOperator");
    // norm!(success_simplifications_and, "simplifications/and");
    // norm!(success_simplifications_eq, "simplifications/eq");
    // norm!(success_simplifications_ifThenElse, "simplifications/ifThenElse");
    // norm!(success_simplifications_ne, "simplifications/ne");
    // norm!(success_simplifications_or, "simplifications/or");


    norm!(success_unit_Bool, "unit/Bool");
    norm!(success_unit_Double, "unit/Double");
    norm!(success_unit_DoubleLiteral, "unit/DoubleLiteral");
    norm!(success_unit_DoubleShow, "unit/DoubleShow");
    // norm!(success_unit_DoubleShowValue, "unit/DoubleShowValue");
    norm!(success_unit_FunctionApplicationCapture, "unit/FunctionApplicationCapture");
    norm!(success_unit_FunctionApplicationNoSubstitute, "unit/FunctionApplicationNoSubstitute");
    norm!(success_unit_FunctionApplicationNormalizeArguments, "unit/FunctionApplicationNormalizeArguments");
    norm!(success_unit_FunctionApplicationSubstitute, "unit/FunctionApplicationSubstitute");
    norm!(success_unit_FunctionNormalizeArguments, "unit/FunctionNormalizeArguments");
    norm!(success_unit_FunctionTypeNormalizeArguments, "unit/FunctionTypeNormalizeArguments");
    // norm!(success_unit_IfAlternativesIdentical, "unit/IfAlternativesIdentical");
    norm!(success_unit_IfFalse, "unit/IfFalse");
    norm!(success_unit_IfNormalizePredicateAndBranches, "unit/IfNormalizePredicateAndBranches");
    // norm!(success_unit_IfTrivial, "unit/IfTrivial");
    norm!(success_unit_IfTrue, "unit/IfTrue");
    norm!(success_unit_Integer, "unit/Integer");
    norm!(success_unit_IntegerNegative, "unit/IntegerNegative");
    norm!(success_unit_IntegerPositive, "unit/IntegerPositive");
    norm!(success_unit_IntegerShow_12, "unit/IntegerShow-12");
    norm!(success_unit_IntegerShow12, "unit/IntegerShow12");
    norm!(success_unit_IntegerShow, "unit/IntegerShow");
    // norm!(success_unit_IntegerToDouble_12, "unit/IntegerToDouble-12");
    // norm!(success_unit_IntegerToDouble12, "unit/IntegerToDouble12");
    norm!(success_unit_IntegerToDouble, "unit/IntegerToDouble");
    norm!(success_unit_Kind, "unit/Kind");
    norm!(success_unit_Let, "unit/Let");
    norm!(success_unit_LetWithType, "unit/LetWithType");
    norm!(success_unit_List, "unit/List");
    norm!(success_unit_ListBuild, "unit/ListBuild");
    norm!(success_unit_ListBuildFoldFusion, "unit/ListBuildFoldFusion");
    norm!(success_unit_ListBuildImplementation, "unit/ListBuildImplementation");
    norm!(success_unit_ListFold, "unit/ListFold");
    norm!(success_unit_ListFoldEmpty, "unit/ListFoldEmpty");
    norm!(success_unit_ListFoldOne, "unit/ListFoldOne");
    norm!(success_unit_ListHead, "unit/ListHead");
    norm!(success_unit_ListHeadEmpty, "unit/ListHeadEmpty");
    norm!(success_unit_ListHeadOne, "unit/ListHeadOne");
    norm!(success_unit_ListIndexed, "unit/ListIndexed");
    norm!(success_unit_ListIndexedEmpty, "unit/ListIndexedEmpty");
    norm!(success_unit_ListIndexedOne, "unit/ListIndexedOne");
    norm!(success_unit_ListLast, "unit/ListLast");
    norm!(success_unit_ListLastEmpty, "unit/ListLastEmpty");
    norm!(success_unit_ListLastOne, "unit/ListLastOne");
    norm!(success_unit_ListLength, "unit/ListLength");
    norm!(success_unit_ListLengthEmpty, "unit/ListLengthEmpty");
    norm!(success_unit_ListLengthOne, "unit/ListLengthOne");
    norm!(success_unit_ListNormalizeElements, "unit/ListNormalizeElements");
    norm!(success_unit_ListNormalizeTypeAnnotation, "unit/ListNormalizeTypeAnnotation");
    norm!(success_unit_ListReverse, "unit/ListReverse");
    norm!(success_unit_ListReverseEmpty, "unit/ListReverseEmpty");
    norm!(success_unit_ListReverseTwo, "unit/ListReverseTwo");
    // norm!(success_unit_Merge, "unit/Merge");
    norm!(success_unit_MergeNormalizeArguments, "unit/MergeNormalizeArguments");
    norm!(success_unit_MergeWithType, "unit/MergeWithType");
    norm!(success_unit_MergeWithTypeNormalizeArguments, "unit/MergeWithTypeNormalizeArguments");
    norm!(success_unit_Natural, "unit/Natural");
    norm!(success_unit_NaturalBuild, "unit/NaturalBuild");
    norm!(success_unit_NaturalBuildFoldFusion, "unit/NaturalBuildFoldFusion");
    norm!(success_unit_NaturalBuildImplementation, "unit/NaturalBuildImplementation");
    norm!(success_unit_NaturalEven, "unit/NaturalEven");
    norm!(success_unit_NaturalEvenOne, "unit/NaturalEvenOne");
    norm!(success_unit_NaturalEvenZero, "unit/NaturalEvenZero");
    norm!(success_unit_NaturalFold, "unit/NaturalFold");
    norm!(success_unit_NaturalFoldOne, "unit/NaturalFoldOne");
    norm!(success_unit_NaturalFoldZero, "unit/NaturalFoldZero");
    norm!(success_unit_NaturalIsZero, "unit/NaturalIsZero");
    norm!(success_unit_NaturalIsZeroOne, "unit/NaturalIsZeroOne");
    norm!(success_unit_NaturalIsZeroZero, "unit/NaturalIsZeroZero");
    norm!(success_unit_NaturalLiteral, "unit/NaturalLiteral");
    norm!(success_unit_NaturalOdd, "unit/NaturalOdd");
    norm!(success_unit_NaturalOddOne, "unit/NaturalOddOne");
    norm!(success_unit_NaturalOddZero, "unit/NaturalOddZero");
    norm!(success_unit_NaturalShow, "unit/NaturalShow");
    norm!(success_unit_NaturalShowOne, "unit/NaturalShowOne");
    norm!(success_unit_NaturalToInteger, "unit/NaturalToInteger");
    norm!(success_unit_NaturalToIntegerOne, "unit/NaturalToIntegerOne");
    norm!(success_unit_None, "unit/None");
    norm!(success_unit_NoneNatural, "unit/NoneNatural");
    // norm!(success_unit_OperatorAndEquivalentArguments, "unit/OperatorAndEquivalentArguments");
    // norm!(success_unit_OperatorAndLhsFalse, "unit/OperatorAndLhsFalse");
    // norm!(success_unit_OperatorAndLhsTrue, "unit/OperatorAndLhsTrue");
    // norm!(success_unit_OperatorAndNormalizeArguments, "unit/OperatorAndNormalizeArguments");
    // norm!(success_unit_OperatorAndRhsFalse, "unit/OperatorAndRhsFalse");
    // norm!(success_unit_OperatorAndRhsTrue, "unit/OperatorAndRhsTrue");
    // norm!(success_unit_OperatorEqualEquivalentArguments, "unit/OperatorEqualEquivalentArguments");
    // norm!(success_unit_OperatorEqualLhsTrue, "unit/OperatorEqualLhsTrue");
    // norm!(success_unit_OperatorEqualNormalizeArguments, "unit/OperatorEqualNormalizeArguments");
    // norm!(success_unit_OperatorEqualRhsTrue, "unit/OperatorEqualRhsTrue");
    norm!(success_unit_OperatorListConcatenateLhsEmpty, "unit/OperatorListConcatenateLhsEmpty");
    norm!(success_unit_OperatorListConcatenateListList, "unit/OperatorListConcatenateListList");
    norm!(success_unit_OperatorListConcatenateNormalizeArguments, "unit/OperatorListConcatenateNormalizeArguments");
    norm!(success_unit_OperatorListConcatenateRhsEmpty, "unit/OperatorListConcatenateRhsEmpty");
    // norm!(success_unit_OperatorNotEqualEquivalentArguments, "unit/OperatorNotEqualEquivalentArguments");
    // norm!(success_unit_OperatorNotEqualLhsFalse, "unit/OperatorNotEqualLhsFalse");
    // norm!(success_unit_OperatorNotEqualNormalizeArguments, "unit/OperatorNotEqualNormalizeArguments");
    // norm!(success_unit_OperatorNotEqualRhsFalse, "unit/OperatorNotEqualRhsFalse");
    // norm!(success_unit_OperatorOrEquivalentArguments, "unit/OperatorOrEquivalentArguments");
    // norm!(success_unit_OperatorOrLhsFalse, "unit/OperatorOrLhsFalse");
    // norm!(success_unit_OperatorOrLhsTrue, "unit/OperatorOrLhsTrue");
    // norm!(success_unit_OperatorOrNormalizeArguments, "unit/OperatorOrNormalizeArguments");
    // norm!(success_unit_OperatorOrRhsFalse, "unit/OperatorOrRhsFalse");
    // norm!(success_unit_OperatorOrRhsTrue, "unit/OperatorOrRhsTrue");
    // norm!(success_unit_OperatorPlusLhsZero, "unit/OperatorPlusLhsZero");
    // norm!(success_unit_OperatorPlusNormalizeArguments, "unit/OperatorPlusNormalizeArguments");
    norm!(success_unit_OperatorPlusOneAndOne, "unit/OperatorPlusOneAndOne");
    // norm!(success_unit_OperatorPlusRhsZero, "unit/OperatorPlusRhsZero");
    // norm!(success_unit_OperatorTextConcatenateLhsEmpty, "unit/OperatorTextConcatenateLhsEmpty");
    // norm!(success_unit_OperatorTextConcatenateNormalizeArguments, "unit/OperatorTextConcatenateNormalizeArguments");
    // norm!(success_unit_OperatorTextConcatenateRhsEmpty, "unit/OperatorTextConcatenateRhsEmpty");
    norm!(success_unit_OperatorTextConcatenateTextText, "unit/OperatorTextConcatenateTextText");
    // norm!(success_unit_OperatorTimesLhsOne, "unit/OperatorTimesLhsOne");
    // norm!(success_unit_OperatorTimesLhsZero, "unit/OperatorTimesLhsZero");
    // norm!(success_unit_OperatorTimesNormalizeArguments, "unit/OperatorTimesNormalizeArguments");
    // norm!(success_unit_OperatorTimesRhsOne, "unit/OperatorTimesRhsOne");
    // norm!(success_unit_OperatorTimesRhsZero, "unit/OperatorTimesRhsZero");
    norm!(success_unit_OperatorTimesTwoAndTwo, "unit/OperatorTimesTwoAndTwo");
    norm!(success_unit_Optional, "unit/Optional");
    norm!(success_unit_OptionalBuild, "unit/OptionalBuild");
    norm!(success_unit_OptionalBuildFoldFusion, "unit/OptionalBuildFoldFusion");
    norm!(success_unit_OptionalBuildImplementation, "unit/OptionalBuildImplementation");
    norm!(success_unit_OptionalFold, "unit/OptionalFold");
    norm!(success_unit_OptionalFoldNone, "unit/OptionalFoldNone");
    norm!(success_unit_OptionalFoldSome, "unit/OptionalFoldSome");
    norm!(success_unit_Record, "unit/Record");
    norm!(success_unit_RecordEmpty, "unit/RecordEmpty");
    norm!(success_unit_RecordProjection, "unit/RecordProjection");
    norm!(success_unit_RecordProjectionEmpty, "unit/RecordProjectionEmpty");
    norm!(success_unit_RecordProjectionNormalizeArguments, "unit/RecordProjectionNormalizeArguments");
    norm!(success_unit_RecordSelection, "unit/RecordSelection");
    norm!(success_unit_RecordSelectionNormalizeArguments, "unit/RecordSelectionNormalizeArguments");
    norm!(success_unit_RecordType, "unit/RecordType");
    norm!(success_unit_RecordTypeEmpty, "unit/RecordTypeEmpty");
    // norm!(success_unit_RecursiveRecordMergeCollision, "unit/RecursiveRecordMergeCollision");
    // norm!(success_unit_RecursiveRecordMergeLhsEmpty, "unit/RecursiveRecordMergeLhsEmpty");
    // norm!(success_unit_RecursiveRecordMergeNoCollision, "unit/RecursiveRecordMergeNoCollision");
    // norm!(success_unit_RecursiveRecordMergeNormalizeArguments, "unit/RecursiveRecordMergeNormalizeArguments");
    // norm!(success_unit_RecursiveRecordMergeRhsEmpty, "unit/RecursiveRecordMergeRhsEmpty");
    // norm!(success_unit_RecursiveRecordTypeMergeCollision, "unit/RecursiveRecordTypeMergeCollision");
    // norm!(success_unit_RecursiveRecordTypeMergeLhsEmpty, "unit/RecursiveRecordTypeMergeLhsEmpty");
    // norm!(success_unit_RecursiveRecordTypeMergeNoCollision, "unit/RecursiveRecordTypeMergeNoCollision");
    // norm!(success_unit_RecursiveRecordTypeMergeNormalizeArguments, "unit/RecursiveRecordTypeMergeNormalizeArguments");
    // norm!(success_unit_RecursiveRecordTypeMergeRhsEmpty, "unit/RecursiveRecordTypeMergeRhsEmpty");
    // norm!(success_unit_RightBiasedRecordMergeCollision, "unit/RightBiasedRecordMergeCollision");
    // norm!(success_unit_RightBiasedRecordMergeLhsEmpty, "unit/RightBiasedRecordMergeLhsEmpty");
    // norm!(success_unit_RightBiasedRecordMergeNoCollision, "unit/RightBiasedRecordMergeNoCollision");
    // norm!(success_unit_RightBiasedRecordMergeNormalizeArguments, "unit/RightBiasedRecordMergeNormalizeArguments");
    // norm!(success_unit_RightBiasedRecordMergeRhsEmpty, "unit/RightBiasedRecordMergeRhsEmpty");
    norm!(success_unit_SomeNormalizeArguments, "unit/SomeNormalizeArguments");
    norm!(success_unit_Sort, "unit/Sort");
    norm!(success_unit_Text, "unit/Text");
    // norm!(success_unit_TextInterpolate, "unit/TextInterpolate");
    norm!(success_unit_TextLiteral, "unit/TextLiteral");
    norm!(success_unit_TextNormalizeInterpolations, "unit/TextNormalizeInterpolations");
    norm!(success_unit_TextShow, "unit/TextShow");
    // norm!(success_unit_TextShowAllEscapes, "unit/TextShowAllEscapes");
    norm!(success_unit_True, "unit/True");
    norm!(success_unit_Type, "unit/Type");
    norm!(success_unit_TypeAnnotation, "unit/TypeAnnotation");
    // norm!(success_unit_UnionNormalizeAlternatives, "unit/UnionNormalizeAlternatives");
    norm!(success_unit_UnionNormalizeArguments, "unit/UnionNormalizeArguments");
    // norm!(success_unit_UnionProjectConstructor, "unit/UnionProjectConstructor");
    // norm!(success_unit_UnionSortAlternatives, "unit/UnionSortAlternatives");
    // norm!(success_unit_UnionType, "unit/UnionType");
    norm!(success_unit_UnionTypeEmpty, "unit/UnionTypeEmpty");
    // norm!(success_unit_UnionTypeNormalizeArguments, "unit/UnionTypeNormalizeArguments");
    norm!(success_unit_Variable, "unit/Variable");
}
