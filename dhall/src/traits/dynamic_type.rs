use crate::expr::*;
use crate::traits::StaticType;
use crate::typecheck::{type_of_const, TypeError, TypeMessage};
use dhall_core::context::Context;
use dhall_core::{Const, ExprF};
use std::borrow::Cow;

pub trait DynamicType {
    fn get_type<'a>(&'a self) -> Result<Cow<'a, Type<'static>>, TypeError>;
}

impl<T: StaticType> DynamicType for T {
    fn get_type<'a>(&'a self) -> Result<Cow<'a, Type<'static>>, TypeError> {
        Ok(Cow::Owned(T::get_static_type()))
    }
}

impl<'a> DynamicType for Type<'a> {
    fn get_type(&self) -> Result<Cow<'_, Type<'static>>, TypeError> {
        match &self.0 {
            TypeInternal::Expr(e) => e.get_type(),
            TypeInternal::Const(c) => Ok(Cow::Owned(type_of_const(*c))),
            TypeInternal::SuperType => Err(TypeError::new(
                &Context::new(),
                dhall_core::rc(ExprF::Const(Const::Sort)),
                TypeMessage::Untyped,
            )),
        }
    }
}

impl<'a> DynamicType for Normalized<'a> {
    fn get_type(&self) -> Result<Cow<'_, Type<'static>>, TypeError> {
        match &self.1 {
            Some(t) => Ok(Cow::Borrowed(t)),
            None => Err(TypeError::new(
                &Context::new(),
                self.0.absurd(),
                TypeMessage::Untyped,
            )),
        }
    }
}

impl<'a> DynamicType for Typed<'a> {
    fn get_type(&self) -> Result<Cow<'_, Type<'static>>, TypeError> {
        match &self.1 {
            Some(t) => Ok(Cow::Borrowed(t)),
            None => Err(TypeError::new(
                &Context::new(),
                self.0.clone(),
                TypeMessage::Untyped,
            )),
        }
    }
}
