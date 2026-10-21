//! The platform-neutral description of a binding module that the readers
//! build from an SDK and the emitters turn into code.

/// The types and globals one platform family binds for a plugin.
#[derive(Debug, Default)]
pub(crate) struct Module {
    /// The transport ID of the generated native plugin.
    pub(crate) plugin: String,
    /// The SDKs and modules the module was generated from.
    pub(crate) source: String,
    /// The native modules or packages the generated native source imports.
    pub(crate) imports: Vec<String>,
    pub(crate) types: Vec<Type>,
    /// Apple global constants and functions.
    pub(crate) globals: Vec<Member>,
    /// Symbols left out, with the reason.
    pub(crate) skipped: Vec<Skipped>,
}

#[derive(Debug)]
pub(crate) struct Skipped {
    pub(crate) symbol: String,
    pub(crate) reason: String,
}

/// A native type.
#[derive(Debug)]
pub(crate) struct Type {
    /// Its name and the names of the types it is nested in, outermost first.
    pub(crate) path: Vec<String>,
    /// How native source names it.
    pub(crate) native: String,
    /// The runtime class name that native objects report for it.
    pub(crate) runtime: String,
    pub(crate) kind: Kind,
    pub(crate) availability: Availability,
    pub(crate) members: Vec<Member>,
    /// Android compile-time constants.
    pub(crate) constants: Vec<Constant>,
}

/// A constant whose value the SDK declares.
#[derive(Debug)]
pub(crate) struct Constant {
    pub(crate) name: String,
    /// The value as a TypeScript literal.
    pub(crate) value: String,
    pub(crate) availability: Availability,
}

impl Type {
    pub(crate) fn name(&self) -> String {
        self.path.join(".")
    }
}

#[derive(Debug)]
pub(crate) enum Kind {
    /// A class, whose instances are native objects. A class that JavaScript
    /// may subclass, such as an Android abstract callback, has callbacks.
    Class {
        superclass: Option<Vec<String>>,
        callbacks: Option<Callbacks>,
    },
    /// An Apple protocol or Android interface: JavaScript implements its
    /// callbacks, and its instances are native objects.
    Protocol { callbacks: Option<Callbacks> },
    /// A value with named fields, which native code can construct from them
    /// when it is constructible.
    Struct {
        fields: Vec<Field>,
        constructible: bool,
    },
    /// A value that is one of its cases, by name.
    Enum { cases: Vec<Case> },
    /// A set of named options.
    Options { options: Vec<Case> },
    /// Another name for a type.
    Alias(Ty),
}

/// The methods JavaScript handlers implement.
#[derive(Debug)]
pub(crate) struct Callbacks {
    pub(crate) methods: Vec<Callback>,
}

#[derive(Debug)]
pub(crate) struct Callback {
    /// The key of the native method it implements.
    pub(crate) key: String,
    /// The handler's name.
    pub(crate) name: String,
    pub(crate) params: Vec<Param>,
    /// The native declaration the implementation overrides or implements.
    pub(crate) declaration: String,
    /// Whether the platform has a default for it, which remains without a handler.
    pub(crate) default: bool,
    /// Apple only: the Objective-C selector of an optional method.
    pub(crate) selector: Option<String>,
    pub(crate) availability: Availability,
}

#[derive(Debug)]
pub(crate) struct Case {
    /// The name JavaScript uses.
    pub(crate) name: String,
    /// How native source names it.
    pub(crate) native: String,
    pub(crate) availability: Availability,
}

/// A callable member, property or field.
#[derive(Debug, Clone)]
pub(crate) struct Member {
    /// The TypeScript name.
    pub(crate) name: String,
    /// The stable key the call travels under.
    pub(crate) key: String,
    pub(crate) kind: MemberKind,
    pub(crate) is_static: bool,
    pub(crate) params: Vec<Param>,
    /// The result, or for a stream its elements.
    pub(crate) result: Ty,
    pub(crate) effects: Effects,
    /// Whether the result is a sequence of `result` values.
    pub(crate) stream: bool,
    pub(crate) availability: Availability,
    /// The native name: method, property or field.
    pub(crate) native: String,
    /// Apple only: each parameter's argument label, `None` for `_`.
    pub(crate) labels: Vec<Option<String>>,
    /// Android only: whether the last parameter takes variable arguments.
    pub(crate) variadic: bool,
}

/// What calling a member may do besides return.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Effects {
    pub(crate) throws: bool,
    /// Whether the call completes asynchronously natively, as a Swift `async` function.
    pub(crate) asynchronous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Initializer,
    Method,
    Getter,
    Setter,
}

/// A struct's field.
#[derive(Debug, Clone)]
pub(crate) struct Field {
    pub(crate) name: String,
    pub(crate) ty: Ty,
    pub(crate) availability: Availability,
}

#[derive(Debug, Clone)]
pub(crate) struct Param {
    pub(crate) name: String,
    pub(crate) ty: Ty,
}

/// A value's type as the binding converts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ty {
    Void,
    Bool,
    /// An integer, with its native spelling.
    Int(String),
    /// A floating point number, with its native spelling.
    Float(String),
    /// An Android `char`.
    Char,
    /// A value JavaScript holds as a string, with its native spelling:
    /// `String`, `URL`, `UUID`, `Locale` or `TimeZone`.
    String(String),
    /// A number native code holds as an object, such as `NSNumber`.
    Number(String),
    Date,
    Bytes,
    Error,
    /// An Objective-C `NSError **` out-parameter, which JavaScript does not
    /// pass: the call rejects with the error it sets.
    OutError,
    /// An Android `Object` or type variable: any value.
    Any,
    Optional(Box<Ty>),
    List(Box<Ty>, Collection),
    /// String keys and values of a type.
    Record(Box<Ty>),
    Named(Named),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Collection {
    /// A Swift array or Android `List`.
    Array,
    Set,
    /// An Android array of the element type.
    JavaArray,
}

/// A reference to a type the module binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    pub(crate) path: Vec<String>,
    /// How native source names it, with any generic arguments.
    pub(crate) native: String,
    pub(crate) kind: NamedKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NamedKind {
    Object,
    Struct,
    Enum,
    Options,
    Alias(Box<Ty>),
}

impl Ty {
    pub(crate) fn optional(self) -> Self {
        match self {
            Self::Optional(_) | Self::Void => self,
            ty => Self::Optional(Box::new(ty)),
        }
    }
}

/// Where a symbol is available.
#[derive(Debug, Clone, Default)]
pub(crate) struct Availability {
    /// Each platform the symbol is available on, with the version that
    /// introduced it when known: `("iOS", Some("14.0"))`, `("Android", Some("31"))`.
    pub(crate) platforms: Vec<(String, Option<String>)>,
    /// The deprecation message, when deprecated.
    pub(crate) deprecated: Option<String>,
}
