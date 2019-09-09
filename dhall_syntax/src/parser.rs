use itertools::Itertools;
use pest::iterators::Pair;
use pest::prec_climber as pcl;
use pest::prec_climber::PrecClimber;
use pest::Parser;
use std::rc::Rc;

use dhall_generated_parser::{DhallParser, Rule};
use pest_consume::{make_parser, parse_children};

use crate::map::{DupTreeMap, DupTreeSet};
use crate::ExprF::*;
use crate::*;

// This file consumes the parse tree generated by pest and turns it into
// our own AST. All those custom macros should eventually moved into
// their own crate because they are quite general and useful. For now they
// are here and hopefully you can figure out how they work.

type ParsedText<E> = InterpolatedText<Expr<E>>;
type ParsedTextContents<E> = InterpolatedTextContents<Expr<E>>;
type ParseInput<'input, 'data> =
    pest_consume::ParseInput<'input, 'data, Rule, Rc<str>>;

pub type ParseError = pest::error::Error<Rule>;
pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug)]
enum Either<A, B> {
    Left(A),
    Right(B),
}

impl crate::Builtin {
    pub fn parse(s: &str) -> Option<Self> {
        use crate::Builtin::*;
        match s {
            "Bool" => Some(Bool),
            "Natural" => Some(Natural),
            "Integer" => Some(Integer),
            "Double" => Some(Double),
            "Text" => Some(Text),
            "List" => Some(List),
            "Optional" => Some(Optional),
            "None" => Some(OptionalNone),
            "Natural/build" => Some(NaturalBuild),
            "Natural/fold" => Some(NaturalFold),
            "Natural/isZero" => Some(NaturalIsZero),
            "Natural/even" => Some(NaturalEven),
            "Natural/odd" => Some(NaturalOdd),
            "Natural/toInteger" => Some(NaturalToInteger),
            "Natural/show" => Some(NaturalShow),
            "Natural/subtract" => Some(NaturalSubtract),
            "Integer/toDouble" => Some(IntegerToDouble),
            "Integer/show" => Some(IntegerShow),
            "Double/show" => Some(DoubleShow),
            "List/build" => Some(ListBuild),
            "List/fold" => Some(ListFold),
            "List/length" => Some(ListLength),
            "List/head" => Some(ListHead),
            "List/last" => Some(ListLast),
            "List/indexed" => Some(ListIndexed),
            "List/reverse" => Some(ListReverse),
            "Optional/fold" => Some(OptionalFold),
            "Optional/build" => Some(OptionalBuild),
            "Text/show" => Some(TextShow),
            _ => None,
        }
    }
}

fn input_to_span(input: ParseInput) -> Span {
    Span::make(input.user_data().clone(), input.as_pair().as_span())
}
fn spanned<E>(input: ParseInput, x: RawExpr<E>) -> Expr<E> {
    Expr::new(x, input_to_span(input))
}
fn spanned_union<E>(span1: Span, span2: Span, x: RawExpr<E>) -> Expr<E> {
    Expr::new(x, span1.union(&span2))
}

// Trim the shared indent off of a vec of lines, as defined by the Dhall semantics of multiline
// literals.
fn trim_indent<E: Clone>(lines: &mut Vec<ParsedText<E>>) {
    let is_indent = |c: char| c == ' ' || c == '\t';

    // There is at least one line so this is safe
    let last_line_head = lines.last().unwrap().head();
    let indent_chars = last_line_head
        .char_indices()
        .take_while(|(_, c)| is_indent(*c));
    let mut min_indent_idx = match indent_chars.last() {
        Some((i, _)) => i,
        // If there is no indent char, then no indent needs to be stripped
        None => return,
    };

    for line in lines.iter() {
        // Ignore empty lines
        if line.is_empty() {
            continue;
        }
        // Take chars from line while they match the current minimum indent.
        let indent_chars = last_line_head[0..=min_indent_idx]
            .char_indices()
            .zip(line.head().chars())
            .take_while(|((_, c1), c2)| c1 == c2);
        match indent_chars.last() {
            Some(((i, _), _)) => min_indent_idx = i,
            // If there is no indent char, then no indent needs to be stripped
            None => return,
        };
    }

    // Remove the shared indent from non-empty lines
    for line in lines.iter_mut() {
        if !line.is_empty() {
            line.head_mut().replace_range(0..=min_indent_idx, "");
        }
    }
}

lazy_static::lazy_static! {
    static ref PRECCLIMBER: PrecClimber<Rule> = {
        use Rule::*;
        // In order of precedence
        let operators = vec![
            import_alt,
            bool_or,
            natural_plus,
            text_append,
            list_append,
            bool_and,
            combine,
            prefer,
            combine_types,
            natural_times,
            bool_eq,
            bool_ne,
            equivalent,
        ];
        PrecClimber::new(
            operators
                .into_iter()
                .map(|op| pcl::Operator::new(op, pcl::Assoc::Left))
                .collect(),
        )
    };
}

struct Parsers;

#[make_parser(Rule)]
impl Parsers {
    fn EOI(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }

    #[alias(label)]
    fn simple_label(input: ParseInput) -> ParseResult<Label> {
        Ok(Label::from(input.as_str()))
    }
    #[alias(label)]
    fn quoted_label(input: ParseInput) -> ParseResult<Label> {
        Ok(Label::from(input.as_str()))
    }

    fn double_quote_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ParsedText<E>> {
        Ok(parse_children!(input;
            [double_quote_chunk(chunks)..] => {
                chunks.collect()
            }
        ))
    }

    fn double_quote_chunk<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ParsedTextContents<E>> {
        Ok(parse_children!(input;
            [expression(e)] => {
                InterpolatedTextContents::Expr(e)
            },
            [double_quote_char(s)] => {
                InterpolatedTextContents::Text(s)
            },
        ))
    }
    #[alias(double_quote_char)]
    fn double_quote_escaped(input: ParseInput) -> ParseResult<String> {
        Ok(match input.as_str() {
            "\"" => "\"".to_owned(),
            "$" => "$".to_owned(),
            "\\" => "\\".to_owned(),
            "/" => "/".to_owned(),
            "b" => "\u{0008}".to_owned(),
            "f" => "\u{000C}".to_owned(),
            "n" => "\n".to_owned(),
            "r" => "\r".to_owned(),
            "t" => "\t".to_owned(),
            // "uXXXX" or "u{XXXXX}"
            s => {
                use std::convert::{TryFrom, TryInto};

                let s = &s[1..];
                let s = if &s[0..1] == "{" {
                    &s[1..s.len() - 1]
                } else {
                    &s[0..s.len()]
                };

                if s.len() > 8 {
                    Err(input.error(format!(
                        "Escape sequences can't have more than 8 chars: \"{}\"",
                        s
                    )))?
                }

                // pad with zeroes
                let s: String = std::iter::repeat('0')
                    .take(8 - s.len())
                    .chain(s.chars())
                    .collect();

                // `s` has length 8, so `bytes` has length 4
                let bytes: &[u8] = &hex::decode(s).unwrap();
                let i = u32::from_be_bytes(bytes.try_into().unwrap());
                let c = char::try_from(i).unwrap();
                match i {
                    0xD800..=0xDFFF => {
                        let c_ecapsed = c.escape_unicode();
                        Err(input.error(format!("Escape sequences can't contain surrogate pairs: \"{}\"", c_ecapsed)))?
                    }
                    0x0FFFE..=0x0FFFF
                    | 0x1FFFE..=0x1FFFF
                    | 0x2FFFE..=0x2FFFF
                    | 0x3FFFE..=0x3FFFF
                    | 0x4FFFE..=0x4FFFF
                    | 0x5FFFE..=0x5FFFF
                    | 0x6FFFE..=0x6FFFF
                    | 0x7FFFE..=0x7FFFF
                    | 0x8FFFE..=0x8FFFF
                    | 0x9FFFE..=0x9FFFF
                    | 0xAFFFE..=0xAFFFF
                    | 0xBFFFE..=0xBFFFF
                    | 0xCFFFE..=0xCFFFF
                    | 0xDFFFE..=0xDFFFF
                    | 0xEFFFE..=0xEFFFF
                    | 0xFFFFE..=0xFFFFF
                    | 0x10_FFFE..=0x10_FFFF => {
                        let c_ecapsed = c.escape_unicode();
                        Err(input.error(format!("Escape sequences can't contain non-characters: \"{}\"", c_ecapsed)))?
                    }
                    _ => {}
                }
                std::iter::once(c).collect()
            }
        })
    }
    fn double_quote_char(input: ParseInput) -> ParseResult<String> {
        Ok(input.as_str().to_owned())
    }

    fn single_quote_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ParsedText<E>> {
        Ok(parse_children!(input;
            [single_quote_continue(lines)] => {
                let newline: ParsedText<E> = "\n".to_string().into();

                // Reverse lines and chars in each line
                let mut lines: Vec<ParsedText<E>> = lines
                    .into_iter()
                    .rev()
                    .map(|l| l.into_iter().rev().collect::<ParsedText<E>>())
                    .collect();

                trim_indent(&mut lines);

                lines
                    .into_iter()
                    .intersperse(newline)
                    .flat_map(InterpolatedText::into_iter)
                    .collect::<ParsedText<E>>()
            }
        ))
    }
    fn single_quote_char<'a>(
        input: ParseInput<'a, '_>,
    ) -> ParseResult<&'a str> {
        Ok(input.as_str())
    }
    #[alias(single_quote_char)]
    fn escaped_quote_pair<'a>(
        _input: ParseInput<'a, '_>,
    ) -> ParseResult<&'a str> {
        Ok("''")
    }
    #[alias(single_quote_char)]
    fn escaped_interpolation<'a>(
        _input: ParseInput<'a, '_>,
    ) -> ParseResult<&'a str> {
        Ok("${")
    }

    // Returns a vec of lines in reversed order, where each line is also in reversed order.
    fn single_quote_continue<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Vec<Vec<ParsedTextContents<E>>>> {
        Ok(parse_children!(input;
            [expression(e), single_quote_continue(lines)] => {
                let c = InterpolatedTextContents::Expr(e);
                let mut lines = lines;
                lines.last_mut().unwrap().push(c);
                lines
            },
            [single_quote_char(c), single_quote_continue(lines)] => {
                let mut lines = lines;
                if c == "\n" || c == "\r\n" {
                    lines.push(vec![]);
                } else {
                    // TODO: don't allocate for every char
                    let c = InterpolatedTextContents::Text(c.to_owned());
                    lines.last_mut().unwrap().push(c);
                }
                lines
            },
            [] => {
                vec![vec![]]
            },
        ))
    }

    #[alias(expression)]
    fn builtin<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        let s = input.as_str();
        let e = match crate::Builtin::parse(s) {
            Some(b) => Builtin(b),
            None => match s {
                "True" => BoolLit(true),
                "False" => BoolLit(false),
                "Type" => Const(crate::Const::Type),
                "Kind" => Const(crate::Const::Kind),
                "Sort" => Const(crate::Const::Sort),
                _ => Err(input.error(format!("Unrecognized builtin: '{}'", s)))?,
            },
        };
        Ok(spanned(input, e))
    }

    #[alias(double_literal)]
    fn NaN(_input: ParseInput) -> ParseResult<core::Double> {
        Ok(std::f64::NAN.into())
    }
    #[alias(double_literal)]
    fn minus_infinity_literal(_input: ParseInput) -> ParseResult<core::Double> {
        Ok(std::f64::NEG_INFINITY.into())
    }
    #[alias(double_literal)]
    fn plus_infinity_literal(_input: ParseInput) -> ParseResult<core::Double> {
        Ok(std::f64::INFINITY.into())
    }

    #[alias(double_literal)]
    fn numeric_double_literal(input: ParseInput) -> ParseResult<core::Double> {
        let s = input.as_str().trim();
        match s.parse::<f64>() {
            Ok(x) if x.is_infinite() => Err(input.error(format!(
                "Overflow while parsing double literal '{}'",
                s
            ))),
            Ok(x) => Ok(NaiveDouble::from(x)),
            Err(e) => Err(input.error(format!("{}", e))),
        }
    }

    fn natural_literal(input: ParseInput) -> ParseResult<core::Natural> {
        input
            .as_str()
            .trim()
            .parse()
            .map_err(|e| input.error(format!("{}", e)))
    }

    fn integer_literal(input: ParseInput) -> ParseResult<core::Integer> {
        input
            .as_str()
            .trim()
            .parse()
            .map_err(|e| input.error(format!("{}", e)))
    }

    #[alias(expression, shortcut = true)]
    fn identifier<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [variable(v)] => {
                spanned(input, Var(v))
            },
            [expression(e)] => e,
        ))
    }

    fn variable(input: ParseInput) -> ParseResult<V<Label>> {
        Ok(parse_children!(input;
            [label(l), natural_literal(idx)] => {
                V(l, idx)
            },
            [label(l)] => {
                V(l, 0)
            },
        ))
    }

    #[alias(path_component)]
    fn unquoted_path_component(input: ParseInput) -> ParseResult<String> {
        Ok(input.as_str().to_string())
    }
    #[alias(path_component)]
    fn quoted_path_component(input: ParseInput) -> ParseResult<String> {
        #[rustfmt::skip]
        const RESERVED: &percent_encoding::AsciiSet =
            &percent_encoding::CONTROLS
            .add(b'=').add(b':').add(b'/').add(b'?')
            .add(b'#').add(b'[').add(b']').add(b'@')
            .add(b'!').add(b'$').add(b'&').add(b'\'')
            .add(b'(').add(b')').add(b'*').add(b'+')
            .add(b',').add(b';');
        Ok(input
            .as_str()
            .chars()
            .map(|c| {
                // Percent-encode ascii chars
                if c.is_ascii() {
                    percent_encoding::utf8_percent_encode(
                        &c.to_string(),
                        RESERVED,
                    )
                    .to_string()
                } else {
                    c.to_string()
                }
            })
            .collect())
    }
    fn path(input: ParseInput) -> ParseResult<Vec<String>> {
        Ok(parse_children!(input;
            [path_component(components)..] => {
                components.collect()
            }
        ))
    }

    #[alias(import_type)]
    fn local<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ImportLocation<Expr<E>>> {
        Ok(parse_children!(input;
            [local_path((prefix, p))] => ImportLocation::Local(prefix, p),
        ))
    }

    #[alias(local_path)]
    fn parent_path(
        input: ParseInput,
    ) -> ParseResult<(FilePrefix, Vec<String>)> {
        Ok(parse_children!(input;
            [path(p)] => (FilePrefix::Parent, p)
        ))
    }
    #[alias(local_path)]
    fn here_path(input: ParseInput) -> ParseResult<(FilePrefix, Vec<String>)> {
        Ok(parse_children!(input;
            [path(p)] => (FilePrefix::Here, p)
        ))
    }
    #[alias(local_path)]
    fn home_path(input: ParseInput) -> ParseResult<(FilePrefix, Vec<String>)> {
        Ok(parse_children!(input;
            [path(p)] => (FilePrefix::Home, p)
        ))
    }
    #[alias(local_path)]
    fn absolute_path(
        input: ParseInput,
    ) -> ParseResult<(FilePrefix, Vec<String>)> {
        Ok(parse_children!(input;
            [path(p)] => (FilePrefix::Absolute, p)
        ))
    }

    fn scheme(input: ParseInput) -> ParseResult<Scheme> {
        Ok(match input.as_str() {
            "http" => Scheme::HTTP,
            "https" => Scheme::HTTPS,
            _ => unreachable!(),
        })
    }

    fn http_raw<E: Clone>(input: ParseInput) -> ParseResult<URL<Expr<E>>> {
        Ok(parse_children!(input;
            [scheme(sch), authority(auth), path(p)] => URL {
                scheme: sch,
                authority: auth,
                path: p,
                query: None,
                headers: None,
            },
            [scheme(sch), authority(auth), path(p), query(q)] => URL {
                scheme: sch,
                authority: auth,
                path: p,
                query: Some(q),
                headers: None,
            },
        ))
    }

    fn authority(input: ParseInput) -> ParseResult<String> {
        Ok(input.as_str().to_owned())
    }

    fn query(input: ParseInput) -> ParseResult<String> {
        Ok(input.as_str().to_owned())
    }

    #[alias(import_type)]
    fn http<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ImportLocation<Expr<E>>> {
        Ok(ImportLocation::Remote(parse_children!(input;
            [http_raw(url)] => url,
            [http_raw(url), expression(e)] => URL { headers: Some(e), ..url },
        )))
    }

    #[alias(import_type)]
    fn env<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<ImportLocation<Expr<E>>> {
        Ok(parse_children!(input;
            [environment_variable(v)] => ImportLocation::Env(v),
        ))
    }
    #[alias(environment_variable)]
    fn bash_environment_variable(input: ParseInput) -> ParseResult<String> {
        Ok(input.as_str().to_owned())
    }
    #[alias(environment_variable)]
    fn posix_environment_variable(input: ParseInput) -> ParseResult<String> {
        Ok(parse_children!(input;
            [posix_environment_variable_character(chars)..] => {
                chars.collect()
            },
        ))
    }
    fn posix_environment_variable_character<'a>(
        input: ParseInput<'a, '_>,
    ) -> ParseResult<&'a str> {
        Ok(match input.as_str() {
            "\\\"" => "\"",
            "\\\\" => "\\",
            "\\a" => "\u{0007}",
            "\\b" => "\u{0008}",
            "\\f" => "\u{000C}",
            "\\n" => "\n",
            "\\r" => "\r",
            "\\t" => "\t",
            "\\v" => "\u{000B}",
            s => s,
        })
    }

    #[alias(import_type)]
    fn missing<E: Clone>(
        _input: ParseInput,
    ) -> ParseResult<ImportLocation<Expr<E>>> {
        Ok(ImportLocation::Missing)
    }

    fn hash(input: ParseInput) -> ParseResult<Hash> {
        let s = input.as_str().trim();
        let protocol = &s[..6];
        let hash = &s[7..];
        if protocol != "sha256" {
            Err(input.error(format!("Unknown hashing protocol '{}'", protocol)))?
        }
        Ok(Hash::SHA256(hex::decode(hash).unwrap()))
    }

    fn import_hashed<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<crate::Import<Expr<E>>> {
        use crate::Import;
        let mode = ImportMode::Code;
        Ok(parse_children!(input;
            [import_type(location)] => Import { mode, location, hash: None },
            [import_type(location), hash(h)] => Import { mode, location, hash: Some(h) },
        ))
    }

    #[alias(import_mode)]
    fn Text(_input: ParseInput) -> ParseResult<ImportMode> {
        Ok(ImportMode::RawText)
    }
    #[alias(import_mode)]
    fn Location(_input: ParseInput) -> ParseResult<ImportMode> {
        Ok(ImportMode::Location)
    }

    #[alias(expression)]
    fn import<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        use crate::Import;
        let import = parse_children!(input;
            [import_hashed(imp)] => {
                Import { mode: ImportMode::Code, ..imp }
            },
            [import_hashed(imp), import_mode(mode)] => {
                Import { mode, ..imp }
            },
        );
        Ok(spanned(input, Import(import)))
    }

    fn lambda(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn forall(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn arrow(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn merge(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn assert(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn if_(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }
    fn toMap(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }

    #[alias(expression)]
    fn empty_list_literal<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [expression(e)] => spanned(input, EmptyListLit(e)),
        ))
    }

    fn expression<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [lambda(()), label(l), expression(typ),
                    arrow(()), expression(body)] => {
                spanned(input, Lam(l, typ, body))
            },
            [if_(()), expression(cond), expression(left),
                    expression(right)] => {
                spanned(input, BoolIf(cond, left, right))
            },
            [let_binding(bindings).., expression(final_expr)] => {
                bindings.rev().fold(
                    final_expr,
                    |acc, x| {
                        spanned_union(
                            acc.span().unwrap(),
                            x.3,
                            Let(x.0, x.1, x.2, acc)
                        )
                    }
                )
            },
            [forall(()), label(l), expression(typ),
                    arrow(()), expression(body)] => {
                spanned(input, Pi(l, typ, body))
            },
            [expression(typ), arrow(()), expression(body)] => {
                spanned(input, Pi("_".into(), typ, body))
            },
            [merge(()), expression(x), expression(y), expression(z)] => {
                spanned(input, Merge(x, y, Some(z)))
            },
            [assert(()), expression(x)] => {
                spanned(input, Assert(x))
            },
            [toMap(()), expression(x), expression(y)] => {
                spanned(input, ToMap(x, Some(y)))
            },
            [expression(e), expression(annot)] => {
                spanned(input, Annot(e, annot))
            },
            [expression(e)] => e,
        ))
    }

    fn let_binding<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Label, Option<Expr<E>>, Expr<E>, Span)> {
        Ok(parse_children!(input;
            [label(name), expression(annot), expression(expr)] =>
                (name, Some(annot), expr, input_to_span(input)),
            [label(name), expression(expr)] =>
                (name, None, expr, input_to_span(input)),
        ))
    }

    #[alias(expression, shortcut = true)]
    #[prec_climb(expression, PRECCLIMBER)]
    fn operator_expression<E: Clone>(
        input: ParseInput,
        l: Expr<E>,
        op: Pair<Rule>,
        r: Expr<E>,
    ) -> ParseResult<Expr<E>> {
        use crate::BinOp::*;
        use Rule::*;
        let op = match op.as_rule() {
            import_alt => ImportAlt,
            bool_or => BoolOr,
            natural_plus => NaturalPlus,
            text_append => TextAppend,
            list_append => ListAppend,
            bool_and => BoolAnd,
            combine => RecursiveRecordMerge,
            prefer => RightBiasedRecordMerge,
            combine_types => RecursiveRecordTypeMerge,
            natural_times => NaturalTimes,
            bool_eq => BoolEQ,
            bool_ne => BoolNE,
            equivalent => Equivalence,
            r => Err(input.error(format!("Rule {:?} isn't an operator", r)))?,
        };

        Ok(spanned_union(
            l.span().unwrap(),
            r.span().unwrap(),
            BinOp(op, l, r),
        ))
    }

    fn Some_(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }

    #[alias(expression, shortcut = true)]
    fn application_expression<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [expression(e)] => e,
            [expression(first), expression(rest)..] => {
                rest.fold(
                    first,
                    |acc, e| {
                        spanned_union(
                            acc.span().unwrap(),
                            e.span().unwrap(),
                            App(acc, e)
                        )
                    }
                )
            },
        ))
    }

    #[alias(expression, shortcut = true)]
    fn first_application_expression<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [Some_(()), expression(e)] => {
                spanned(input, SomeLit(e))
            },
            [merge(()), expression(x), expression(y)] => {
                spanned(input, Merge(x, y, None))
            },
            [toMap(()), expression(x)] => {
                spanned(input, ToMap(x, None))
            },
            [expression(e)] => e,
        ))
    }

    #[alias(expression, shortcut = true)]
    fn selector_expression<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [expression(e)] => e,
            [expression(first), selector(rest)..] => {
                rest.fold(
                    first,
                    |acc, e| {
                        spanned_union(
                            acc.span().unwrap(),
                            e.1,
                            match e.0 {
                                Either::Left(l) => Field(acc, l),
                                Either::Right(ls) => Projection(acc, ls),
                            }
                        )
                    }
                )
            },
        ))
    }

    fn selector(
        input: ParseInput,
    ) -> ParseResult<(Either<Label, DupTreeSet<Label>>, Span)> {
        Ok(parse_children!(input;
            [label(l)] => (Either::Left(l), input_to_span(input)),
            [labels(ls)] => (Either::Right(ls), input_to_span(input)),
            // [expression(_e)] => unimplemented!("selection by expression"), // TODO
        ))
    }

    fn labels(input: ParseInput) -> ParseResult<DupTreeSet<Label>> {
        Ok(parse_children!(input;
            [label(ls)..] => ls.collect(),
        ))
    }

    #[alias(expression, shortcut = true)]
    fn primitive_expression<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [double_literal(n)] => spanned(input, DoubleLit(n)),
            [natural_literal(n)] => spanned(input, NaturalLit(n)),
            [integer_literal(n)] => spanned(input, IntegerLit(n)),
            [double_quote_literal(s)] => spanned(input, TextLit(s)),
            [single_quote_literal(s)] => spanned(input, TextLit(s)),
            [expression(e)] => e,
        ))
    }

    #[alias(expression)]
    fn empty_record_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(spanned(input, RecordLit(Default::default())))
    }

    #[alias(expression)]
    fn empty_record_type<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        Ok(spanned(input, RecordType(Default::default())))
    }

    #[alias(expression)]
    fn non_empty_record_type_or_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        let e = parse_children!(input;
            [label(first_label), non_empty_record_type(rest)] => {
                let (first_expr, mut map) = rest;
                map.insert(first_label, first_expr);
                RecordType(map)
            },
            [label(first_label), non_empty_record_literal(rest)] => {
                let (first_expr, mut map) = rest;
                map.insert(first_label, first_expr);
                RecordLit(map)
            },
        );
        Ok(spanned(input, e))
    }

    fn non_empty_record_type<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Expr<E>, DupTreeMap<Label, Expr<E>>)> {
        Ok(parse_children!(input;
            [expression(expr), record_type_entry(entries)..] => {
                (expr, entries.collect())
            }
        ))
    }

    fn record_type_entry<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Label, Expr<E>)> {
        Ok(parse_children!(input;
            [label(name), expression(expr)] => (name, expr)
        ))
    }

    fn non_empty_record_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Expr<E>, DupTreeMap<Label, Expr<E>>)> {
        Ok(parse_children!(input;
            [expression(expr), record_literal_entry(entries)..] => {
                (expr, entries.collect())
            }
        ))
    }

    fn record_literal_entry<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Label, Expr<E>)> {
        Ok(parse_children!(input;
            [label(name), expression(expr)] => (name, expr)
        ))
    }

    #[alias(expression)]
    fn union_type<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        let map = parse_children!(input;
            [empty_union_type(_)] => Default::default(),
            [union_type_entry(entries)..] => entries.collect(),
        );
        Ok(spanned(input, UnionType(map)))
    }

    fn empty_union_type(_input: ParseInput) -> ParseResult<()> {
        Ok(())
    }

    fn union_type_entry<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<(Label, Option<Expr<E>>)> {
        Ok(parse_children!(input;
            [label(name), expression(expr)] => (name, Some(expr)),
            [label(name)] => (name, None),
        ))
    }

    #[alias(expression)]
    fn non_empty_list_literal<E: Clone>(
        input: ParseInput,
    ) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [expression(items)..] => spanned(
                input,
                NEListLit(items.collect())
            )
        ))
    }

    fn final_expression<E: Clone>(input: ParseInput) -> ParseResult<Expr<E>> {
        Ok(parse_children!(input;
            [expression(e), EOI(_)] => e
        ))
    }
}

pub fn parse_expr<E: Clone>(input_str: &str) -> ParseResult<Expr<E>> {
    let mut pairs = DhallParser::parse(Rule::final_expression, input_str)?;
    // TODO: proper errors
    let pair = pairs.next().unwrap();
    assert_eq!(pairs.next(), None);
    let rc_input_str = input_str.to_string().into();
    let input = ParseInput::new(pair, &rc_input_str);
    Parsers::final_expression(input)
}
