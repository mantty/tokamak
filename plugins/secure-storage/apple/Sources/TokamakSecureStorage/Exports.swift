import TokamakPlugin

extension TokamakSecureStoragePlugin: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<TokamakSecureStoragePlugin>) {
    e.call(
      create, doc: "Stores a new value; rejects with `DuplicateItemError` when `name` has one.")
    e.call(
      replace, doc: "Replaces the value in place; rejects with `ItemNotFoundError` without one.")
    e.call(set, doc: "Replaces the value in place, or stores a new one when `name` has none.")
    e.call(get, doc: "Reads the value; rejects with `ItemNotFoundError` when `name` has none.")
    e.call(delete, doc: "Deletes the value; rejects with `ItemNotFoundError` when `name` has none.")
    e.call(clear, doc: "Deletes every stored value; rejects with `ItemNotFoundError` without any.")
  }
}
