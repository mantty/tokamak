package com.tokamak.plugins.securestorage

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports

object SecureStorageExports : TokamakExports<TokamakSecureStoragePlugin> {
    override fun exports(e: TokamakExporter<TokamakSecureStoragePlugin>) {
        e.call(TokamakSecureStoragePlugin::create, doc = "Stores a new value; rejects with `DuplicateItemError` when `name` has one.")
        e.call(TokamakSecureStoragePlugin::replace, doc = "Replaces the value in place; rejects with `ItemNotFoundError` without one.")
        e.call(TokamakSecureStoragePlugin::set, doc = "Replaces the value in place, or stores a new one when `name` has none.")
        e.call(TokamakSecureStoragePlugin::get, doc = "Reads the value; rejects with `ItemNotFoundError` when `name` has none.")
        e.call(TokamakSecureStoragePlugin::delete, doc = "Deletes the value; rejects with `ItemNotFoundError` when `name` has none.")
        e.call(TokamakSecureStoragePlugin::clear, doc = "Deletes every stored value; rejects with `ItemNotFoundError` without any.")
    }
}
