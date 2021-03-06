# `dhall-rust`

[![Build Status](https://travis-ci.org/Nadrieril/dhall-rust.svg?branch=master)](https://travis-ci.org/Nadrieril/dhall-rust)
[![codecov](https://codecov.io/gh/Nadrieril/dhall-rust/branch/master/graph/badge.svg)](https://codecov.io/gh/Nadrieril/dhall-rust)

This is a WIP implementation in Rust of the [dhall](https://dhall-lang.org) configuration format/programming language.

This language is defined by a [standard](https://github.com/dhall-lang/dhall-lang), and this implementation tries its best to respect it.

This is still quite unstable so use at your own risk. Documentation is severely lacking for now, sorry !

## Standard-compliance

- Parsing: 100%
- Imports: 0%
- Normalization: 74%
- Typechecking: 66%

You can see what's missing from the commented out tests in `dhall/tests`.

