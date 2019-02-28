#![feature(box_patterns)]

pub mod context;
mod core;
pub use crate::core::*;
use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub grammar); // synthesized by LALRPOP
mod grammar_util;
mod generated_parser;
pub mod lexer;
pub mod parser;
pub mod typecheck;
