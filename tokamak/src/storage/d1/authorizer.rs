//! D1's SQL authorizer: what workerd's `SqliteDatabase::isAuthorized` allows
//! under the `SqlStorageRegulator` that local D1 runs queries with, at workerd
//! v1.20260825.1 (`src/workerd/util/sqlite.c++`, `src/workerd/api/sql.c++`).

use rusqlite::hooks::{AuthAction, AuthContext};

/// workerd's message for a refused `BEGIN` or `SAVEPOINT`.
pub(super) const TRANSACTION_REFUSED: &str = "To execute a transaction, please use the \
    state.storage.transaction() or state.storage.transactionSync() APIs instead of the SQL \
    BEGIN TRANSACTION or SAVEPOINT statements. The JavaScript API is safer because it will \
    automatically roll back on exceptions, and because it interacts correctly with Durable \
    Objects' automatic atomic write coalescing.";

/// Whether D1 permits an action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Decision {
    Allow,
    Deny,
    /// Denied with a message that replaces SQLite's "not authorized".
    Refuse(&'static str),
}

impl From<bool> for Decision {
    fn from(allowed: bool) -> Self {
        if allowed { Self::Allow } else { Self::Deny }
    }
}

/// Decide an action a statement being prepared needs.
pub(super) fn authorize(context: &AuthContext<'_>) -> Decision {
    // SQLite passes the database name in the first argument of these two, and
    // workerd's patch passes a rename's destination where the name would be.
    let (database, destination) = match context.action {
        AuthAction::AlterTable { database_name, .. } => {
            (Some(database_name), context.database_name)
        }
        AuthAction::Detach { database_name } => (Some(database_name), None),
        _ => (context.database_name, None),
    };
    match database {
        Some("temp") => return authorize_temporary(&context.action).into(),
        Some(name) if name != "main" => return Decision::Deny,
        _ => {}
    }
    authorize_main(&context.action, destination)
}

fn authorize_main(action: &AuthAction<'_>, destination: Option<&str>) -> Decision {
    match *action {
        AuthAction::Select | AuthAction::Analyze { .. } | AuthAction::Recursive => Decision::Allow,
        AuthAction::CreateTable { table_name: name }
        | AuthAction::Delete { table_name: name }
        | AuthAction::DropTable { table_name: name }
        | AuthAction::Insert { table_name: name }
        | AuthAction::CreateView { view_name: name }
        | AuthAction::DropView { view_name: name }
        | AuthAction::Reindex { index_name: name }
        | AuthAction::Read {
            table_name: name, ..
        }
        | AuthAction::Update {
            table_name: name, ..
        } => allowed_name(name).into(),
        AuthAction::AlterTable { table_name, .. } => {
            (allowed_name(table_name) && destination.is_none_or(allowed_name)).into()
        }
        AuthAction::CreateIndex {
            index_name: first,
            table_name: second,
        }
        | AuthAction::DropIndex {
            index_name: first,
            table_name: second,
        }
        | AuthAction::CreateTrigger {
            trigger_name: first,
            table_name: second,
        }
        | AuthAction::DropTrigger {
            trigger_name: first,
            table_name: second,
        } => (allowed_name(first) && allowed_name(second)).into(),
        AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
            Decision::Refuse(TRANSACTION_REFUSED)
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } => allowed_pragma(pragma_name, pragma_value).into(),
        AuthAction::Function { function_name } => FUNCTIONS.contains(&function_name).into(),
        AuthAction::CreateVtable {
            table_name,
            module_name,
        }
        | AuthAction::DropVtable {
            table_name,
            module_name,
        } => (MODULES
            .iter()
            .any(|module| module.eq_ignore_ascii_case(module_name))
            && allowed_name(table_name))
        .into(),
        _ => Decision::Deny,
    }
}

/// The temporary database admits only reads and writes of existing names.
fn authorize_temporary(action: &AuthAction<'_>) -> bool {
    match *action {
        AuthAction::Read { table_name, .. } | AuthAction::Update { table_name, .. } => {
            allowed_name(table_name)
        }
        _ => false,
    }
}

/// Names beginning `_cf_`, in any case, belong to the platform.
fn allowed_name(name: &str) -> bool {
    !name
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("_cf_"))
}

fn allowed_pragma(pragma: &str, argument: Option<&str>) -> bool {
    match pragma {
        "table_list" => return true,
        "table_info" | "table_xinfo" => return argument.is_some_and(allowed_name),
        _ => {}
    }
    let Some((_, signature)) = PRAGMAS.iter().find(|(name, _)| *name == pragma) else {
        return false;
    };
    let Some(argument) = argument else {
        return !matches!(signature, Pragma::ObjectName);
    };
    match signature {
        Pragma::NoArgument => false,
        Pragma::Boolean => boolean(argument),
        Pragma::ObjectName | Pragma::OptionalObjectName => allowed_name(argument),
        Pragma::OptionalNumber => argument.trim_start().parse::<i32>().is_ok(),
        Pragma::OptionalNumberOrObjectName => {
            argument.trim_start().parse::<u32>().is_ok() || allowed_name(argument)
        }
    }
}

/// A boolean in any of SQLite's spellings, optionally quoted, compared as
/// workerd does: by prefix, ignoring case.
fn boolean(argument: &str) -> bool {
    let unquoted = ['\'', '"']
        .into_iter()
        .find_map(|quote| {
            argument
                .strip_prefix(quote)
                .and_then(|value| value.strip_suffix(quote))
        })
        .unwrap_or(argument);
    ["true", "false", "yes", "no", "on", "off", "1", "0"]
        .into_iter()
        .any(|word| {
            unquoted
                .get(..word.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(word))
        })
}

/// The arguments a permitted pragma takes.
#[derive(Clone, Copy)]
enum Pragma {
    NoArgument,
    Boolean,
    ObjectName,
    OptionalObjectName,
    OptionalNumber,
    OptionalNumberOrObjectName,
}

const PRAGMAS: &[(&str, Pragma)] = &[
    ("data_version", Pragma::NoArgument),
    ("page_size", Pragma::NoArgument),
    ("case_sensitive_like", Pragma::Boolean),
    ("foreign_keys", Pragma::Boolean),
    ("defer_foreign_keys", Pragma::Boolean),
    ("ignore_check_constraints", Pragma::Boolean),
    ("legacy_alter_table", Pragma::Boolean),
    ("recursive_triggers", Pragma::Boolean),
    ("reverse_unordered_selects", Pragma::Boolean),
    ("foreign_key_check", Pragma::OptionalObjectName),
    ("foreign_key_list", Pragma::ObjectName),
    ("index_info", Pragma::ObjectName),
    ("index_list", Pragma::ObjectName),
    ("index_xinfo", Pragma::ObjectName),
    ("quick_check", Pragma::OptionalNumberOrObjectName),
    ("optimize", Pragma::OptionalNumber),
];

/// Virtual table modules D1 supports.
const MODULES: [&str; 4] = ["fts5", "fts5vocab", "rtree", "rtree_i32"];

/// SQL functions D1 permits.
const FUNCTIONS: &[&str] = &[
    // Core functions.
    "abs",
    "changes",
    "char",
    "coalesce",
    "concat",
    "concat_ws",
    "format",
    "glob",
    "hex",
    "ifnull",
    "iif",
    "instr",
    "last_insert_rowid",
    "length",
    "like",
    "likelihood",
    "likely",
    "load_extension",
    "lower",
    "ltrim",
    "max_scalar",
    "min_scalar",
    "nullif",
    "octet_length",
    "printf",
    "quote",
    "random",
    "randomblob",
    "replace",
    "round",
    "rtrim",
    "sign",
    "soundex",
    "substr",
    "substring",
    "total_changes",
    "trim",
    "typeof",
    "unhex",
    "unicode",
    "unlikely",
    "upper",
    "zeroblob",
    // Date and time functions.
    "date",
    "time",
    "datetime",
    "julianday",
    "unixepoch",
    "strftime",
    "timediff",
    "current_date",
    "current_time",
    "current_timestamp",
    // Aggregate functions.
    "avg",
    "count",
    "group_concat",
    "max",
    "min",
    "string_agg",
    "sum",
    "total",
    // Window functions.
    "row_number",
    "rank",
    "dense_rank",
    "percent_rank",
    "cume_dist",
    "ntile",
    "lag",
    "lead",
    "first_value",
    "last_value",
    "nth_value",
    // Math functions.
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceil",
    "cos",
    "cosh",
    "degrees",
    "exp",
    "floor",
    "ln",
    "log",
    "log2",
    "mod",
    "pi",
    "pow",
    "radians",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
    "trunc",
    // JSON functions.
    "json",
    "jsonb",
    "json_array",
    "jsonb_array",
    "json_array_length",
    "json_extract",
    "jsonb_extract",
    "->",
    "->>",
    "json_insert",
    "jsonb_insert",
    "json_object",
    "jsonb_object",
    "json_patch",
    "jsonb_patch",
    "json_remove",
    "jsonb_remove",
    "json_replace",
    "jsonb_replace",
    "json_set",
    "jsonb_set",
    "json_type",
    "json_valid",
    "json_quote",
    "json_group_array",
    "jsonb_group_array",
    "json_group_object",
    "jsonb_group_object",
    "json_each",
    "json_tree",
    // FTS5 functions.
    "match",
    "highlight",
    "bm25",
    "snippet",
    // R-Tree functions.
    "rtreecheck",
    // Functions ALTER TABLE uses.
    "sqlite_rename_column",
    "sqlite_rename_table",
    "sqlite_rename_test",
    "sqlite_drop_column",
    "sqlite_rename_quotefix",
];

#[cfg(test)]
mod tests {
    use super::{allowed_name, allowed_pragma, boolean};

    #[test]
    fn reserves_names_beginning_cf_in_any_case() {
        assert!(allowed_name("users"));
        assert!(allowed_name("_cf"));
        assert!(allowed_name("cf_users"));
        assert!(!allowed_name("_cf_KV"));
        assert!(!allowed_name("_CF_metadata"));
    }

    #[test]
    fn permits_introspection_pragmas_for_allowed_names() {
        assert!(allowed_pragma("table_list", None));
        assert!(allowed_pragma("table_info", Some("users")));
        assert!(!allowed_pragma("table_info", Some("_cf_KV")));
        assert!(!allowed_pragma("index_list", None));
        assert!(!allowed_pragma("TABLE_LIST", None));
    }

    #[test]
    fn permits_pragmas_with_their_argument_forms() {
        assert!(allowed_pragma("page_size", None));
        assert!(!allowed_pragma("page_size", Some("4096")));
        assert!(allowed_pragma("foreign_keys", Some("OFF")));
        assert!(allowed_pragma("foreign_keys", None));
        assert!(!allowed_pragma("foreign_keys", Some("maybe")));
        assert!(allowed_pragma("quick_check", Some("10")));
        assert!(allowed_pragma("optimize", None));
        assert!(allowed_pragma("optimize", Some("65538")));
        assert!(!allowed_pragma("optimize", Some("all")));
        assert!(!allowed_pragma("journal_mode", Some("delete")));
    }

    #[test]
    fn reads_booleans_as_workerd_does() {
        for value in ["1", "0", "ON", "'yes'", "\"false\"", "trueish", "Offline"] {
            assert!(boolean(value), "{value}");
        }
        for value in ["", "2", "maybe", "'tr'"] {
            assert!(!boolean(value), "{value}");
        }
    }
}
