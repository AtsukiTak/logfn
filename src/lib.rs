//! This crate provides the `logfn` attribute macro for inserting logging code into your function.
//!
//! Currently we support 2 types of logging.
//!
//! - Pre logging
//! - Post logging
//!
//! And we have a plan to add `Time logging` type.
//!
//! Each `logfn` attribute injects a single logging code. You can put as many `logfn` as you want.
//!
//! ```
//! # use logfn::logfn;
//! # use std::num::ParseIntError;
//! #[logfn(Pre, Debug, "\"atoi\" will be executed")]
//! #[logfn(Post, Debug, "\"atoi\" is executed", if = "Result::is_ok")]
//! #[logfn(Post, Error, "Error \"atoi\": {:?}", if = "Result::is_err")]
//! fn atoi(a: &str) -> Result<usize, ParseIntError> {
//!     usize::from_str_radix(a, 10)
//! }
//! ```
//!
//! The detail is documented below.
//!
//! # Pre logging
//!
//! The following attribute injects logging code **before** function is called.
//!
//! ```
//! use logfn::logfn;
//!
//! #[logfn(Pre, Info, "executing \"add\" function...")]
//! fn add(a: usize, b: usize) -> usize {
//!     a + b
//! }
//! ```
//!
//! The resulting code will looks like
//!
//! ```
//! fn add(a: usize, b: usize) -> usize {
//!     log::info!("executing \"add\" function...");
//!
//!     {
//!         a + b
//!     }
//! }
//! ```
//!
//! # Post logging
//!
//! You also be able to inject logging code **after** function is called.
//!
//! ```
//! use logfn::logfn;
//!
//! #[logfn(Post, Info, "executed \"add\" function!")]
//! fn add(a: usize, b: usize) -> usize {
//!     a + b
//! }
//! ```
//!
//! The resulting code will looks like
//!
//! ```
//! fn add(a: usize, b: usize) -> usize {
//!     let res = (move || {
//!         a + b
//!     })();
//!
//!     log::info!("executed \"add\" function!");
//!
//!     res
//! }
//! ```
//!
//! # Conditional logging
//!
//! You can configure the condition on which logging code is fired.
//! To do that, please add `if` argument with a path to the function which takes reference to
//! returned value and returns `true` when you want to fire the logging code.
//!
//! ## Note
//! Conditional logging is only supported in post logging for now.
//!
//! ```
//! use logfn::logfn;
//!
//! #[logfn(Post, Warn, "checked add is failed!!", if = "Option::is_none")]
//! fn checked_add(a: usize, b: usize) -> Option<usize> {
//!     a.checked_add(b)
//! }
//! ```
//!
//! # Log message
//!
//! You can put a single "{:?}" formatter on post logging. Returned value will be injected.
//!
//! ```
//! # use std::num::ParseIntError;
//! use logfn::logfn;
//!
//! #[logfn(Post, Error, "Error while \"atoi\" function: {:?}", if = "Result::is_err")]
//! fn atoi(s: &str) -> Result<usize, ParseIntError> {
//!     usize::from_str_radix(s, 10)
//! }
//! ```
//!
//! # Async function
//!
//! Async function is also supported.
//!
//! ```
//! use logfn::logfn;
//!
//! #[logfn(Post, Debug, "\"add_fut\" is executed")]
//! async fn add_fut(a: usize, b: usize) -> usize {
//!     a + b
//! }
//! ```
extern crate proc_macro;

mod arg;
mod config;

use proc_macro::TokenStream as StdTokenStream;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, ToTokens};

use config::Config;

macro_rules! try_ok {
    ($x:expr) => {
        match $x {
            Ok(ok) => ok,
            Err(e) => return e.to_compile_error().into(),
        }
    };
}

#[proc_macro_attribute]
pub fn logfn(input_args: StdTokenStream, input: StdTokenStream) -> StdTokenStream {
    // parse config
    let input_args = syn::parse_macro_input!(input_args as syn::AttributeArgs);
    let args = try_ok!(arg::from_input_vec(input_args));
    let config = try_ok!(config::from_args(args));

    // parse input
    let input = syn::parse_macro_input!(input as syn::ItemFn);

    produce_logfn(config, input).into()
}

fn produce_logfn(config: Config, input: syn::ItemFn) -> TokenStream {
    match config.typ {
        arg::TypeArg::Pre => produce_logfn_pre(config, input),
        arg::TypeArg::Post => produce_logfn_post(config, input),
    }
}

// This produces
//
// ```rust
// pub fn add(a: usize, b: usize) -> usize {
//     log::info!();
//
//     {
//         a + b
//     }
// }
// ```
fn produce_logfn_pre(config: Config, input: syn::ItemFn) -> TokenStream {
    let attrs = input.attrs;
    let vis = input.vis;
    let sig = input.sig;
    let block = input.block;

    let log_stmt = produce_log_stmt(&config);

    quote! {
        #(#attrs)*
        #vis #sig {
            #log_stmt

            #block
        }
    }
}

// This produces
//
// ```rust
// pub fn add(a: usize, b: usize) -> usize {
//     fn __logfn_inner(a: usize, b: usize) -> usize {
//         a + b
//     }
//     let res = __logfn_inner();
//
//     log::log!(log::Level::Info, "hoge");
//
//     res
// }
// ```
fn produce_logfn_post(config: Config, input: syn::ItemFn) -> TokenStream {
    let attrs = &input.attrs;
    let vis = &input.vis;
    let sig = &input.sig;

    let closure_call = produce_closure_call(&input);

    let log_stmt = produce_log_stmt(&config);

    let cond_expr = config
        .cond
        .map(|cond| {
            let path = cond.path;
            quote! { #path(&res) }
        })
        .unwrap_or(quote! { true });

    quote! {
        #(#attrs)*
        #vis #sig {
            let res = #closure_call;

            if #cond_expr {
                #log_stmt
            }

            res
        }
    }
}

fn produce_closure_call(input: &syn::ItemFn) -> TokenStream {
    let block = &input.block;

    if input.sig.asyncness.is_some() {
        quote! {
            (move || async move #block)().await
        }
    } else {
        quote! {
            (move || #block)()
        }
    }
}

fn produce_log_stmt(config: &Config) -> TokenStream {
    let log_level = config.level.ident();
    let log_msg = &config.msg.msg;

    if log_msg.contains("{:?}") {
        match config.typ {
            arg::TypeArg::Post => {
                quote! {
                    log::log!(log::Level::#log_level, #log_msg, res);
                }
            }
            _ => todo!("We haven't yet supported this format"),
        }
    } else {
        quote! {
            log::log!(log::Level::#log_level, #log_msg);
        }
    }
}
