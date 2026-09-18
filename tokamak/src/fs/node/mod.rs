#![deny(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::elidable_lifetime_names)]
#![allow(clippy::needless_pass_by_value)]

//! Native `node:fs` bindings for the tokamak `QuickJS` runtime.

use std::sync::{Arc, Mutex};

use crate::fs::vfs::{
    CopyOptions, DirectoryEntry, Error as VfsError, ErrorKind, MAX_PATH_LENGTH, MAX_PATH_SEGMENTS,
    NodeType, OpenOptions, Result as VfsResult, Stat, VirtualFileSystem, join_child, lock,
};
use rquickjs::function::{IntoJsFunc, Opt, Rest, This};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    Array, ArrayBuffer, BigInt, Constructor, Ctx, Exception, Function, Object, Promise, Symbol,
    TypedArray, Value,
};
use rquickjs::{Coerced, FromJs, IntoJs};

mod callback_api;
mod exports;
mod filesystem_operations;
mod javascript_objects;
#[cfg(test)]
mod tests;

#[allow(clippy::wildcard_imports)]
use callback_api::*;
#[allow(clippy::wildcard_imports)]
use filesystem_operations::*;
#[allow(clippy::wildcard_imports)]
use javascript_objects::*;

pub use exports::{MODULE_NAME, NodeFsModule, NodeFsPromisesModule, PROMISES_MODULE_NAME, install};
use exports::{VfsHandle, VfsUserData};
