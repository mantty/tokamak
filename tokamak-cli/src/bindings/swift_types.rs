//! Swift names and types from symbol graph declaration fragments.

use super::apple::{Fragment, Index, Symbol};
use super::model::{Collection, Effects, Named, NamedKind, Ty, capitalize};

/// The effects a declaration has outside its parameter list.
pub(super) fn effects(fragments: &[Fragment]) -> Effects {
    let mut depth = 0_i32;
    let mut effects = Effects {
        throws: false,
        asynchronous: false,
    };
    for fragment in fragments {
        if fragment.kind == "keyword" && depth == 0 {
            match fragment.spelling.as_str() {
                "throws" | "rethrows" => effects.throws = true,
                "async" => effects.asynchronous = true,
                _ => {}
            }
        }
        depth += count(&fragment.spelling, '(') - count(&fragment.spelling, ')');
    }
    effects
}

fn count(text: &str, character: char) -> i32 {
    i32::try_from(text.matches(character).count()).unwrap_or(i32::MAX)
}

/// The TypeScript name of the member `component`: its base name followed by
/// each argument label, capitalised. `locationManager(_:didUpdateLocations:)`
/// is `locationManagerDidUpdateLocations`.
pub(super) fn member_name(component: &str) -> String {
    let base = component.split('(').next().unwrap_or_default();
    labels(component)
        .into_iter()
        .flatten()
        .fold(base.to_owned(), |name, label| name + &capitalize(&label))
}

/// The TypeScript name of the Objective-C `selector`: its parts, each after
/// the first capitalised. `locationManager:didUpdateLocations:` is
/// `locationManagerDidUpdateLocations`.
pub(super) fn selector_name(selector: &str) -> String {
    let mut parts = selector.split(':').filter(|part| !part.is_empty());
    let first = parts.next().unwrap_or_default().to_owned();
    parts.fold(first, |name, part| name + &capitalize(part))
}

/// The argument labels of the member `component`, `None` for `_`.
pub(super) fn labels(component: &str) -> Vec<Option<String>> {
    let Some((_, labels)) = component.split_once('(') else {
        return Vec::new();
    };
    labels
        .trim_end_matches(')')
        .split(':')
        .filter(|label| !label.is_empty())
        .map(|label| (label != "_").then(|| label.to_owned()))
        .collect()
}

/// The declaration's source text.
pub(super) fn text(fragments: &[Fragment]) -> String {
    fragments
        .iter()
        .map(|fragment| fragment.spelling.as_str())
        .collect()
}

/// A protocol requirement's declaration, as an implementation declares it.
pub(super) fn declaration(fragments: &[Fragment]) -> String {
    text(fragments).trim_start_matches("optional ").to_owned()
}

/// The type of a property, from its declaration.
pub(super) fn property_type(index: &Index, fragments: &[Fragment]) -> Result<Ty, String> {
    let tokens = tokens(fragments);
    let start = tokens
        .iter()
        .position(|token| *token == Token::Punct(':'))
        .ok_or("no type")?;
    Parser {
        index,
        tokens: &tokens[start + 1..],
        position: 0,
    }
    .whole(&['{'])
}

/// The type of a function parameter, from its declaration.
pub(super) fn parameter_type(index: &Index, fragments: &[Fragment]) -> Result<Ty, String> {
    let tokens = tokens(fragments);
    let start = tokens
        .iter()
        .position(|token| *token == Token::Punct(':'))
        .map_or(0, |colon| colon + 1);
    Parser {
        index,
        tokens: &tokens[start..],
        position: 0,
    }
    .whole(&['='])
}

/// The type the fragments spell.
pub(super) fn parse(index: &Index, fragments: &[Fragment]) -> Result<Ty, String> {
    let tokens = tokens(fragments);
    if tokens.is_empty() {
        return Ok(Ty::Void);
    }
    Parser {
        index,
        tokens: &tokens,
        position: 0,
    }
    .whole(&[])
}

/// The type a typealias names, spelled with the alias where numbers keep
/// its name natively.
pub(super) fn alias_target(index: &Index, symbol: &Symbol) -> Result<Ty, String> {
    let tokens = tokens(&symbol.declaration_fragments);
    let start = tokens
        .iter()
        .position(|token| *token == Token::Punct('='))
        .ok_or("no aliased type")?;
    let target = Parser {
        index,
        tokens: &tokens[start + 1..],
        position: 0,
    }
    .whole(&[])?;
    let native = symbol.path_components.join(".");
    Ok(match target {
        Ty::Int(_) => Ty::Int(native),
        Ty::Float(_) => Ty::Float(native),
        target => target,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Name {
        spelling: String,
        usr: Option<String>,
    },
    Punct(char),
    Word(String),
}

const PUNCTUATION: &str = "[]:?!<>,().={}";

fn tokens(fragments: &[Fragment]) -> Vec<Token> {
    let mut tokens = Vec::new();
    for fragment in fragments {
        if fragment.kind == "typeIdentifier" {
            tokens.push(Token::Name {
                spelling: fragment.spelling.clone(),
                usr: fragment.precise_identifier.clone(),
            });
        } else {
            split(&fragment.spelling, &mut tokens);
        }
    }
    tokens
}

fn split(text: &str, tokens: &mut Vec<Token>) {
    let mut word = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let arrow = character == '-' && characters.peek() == Some(&'>');
        if (character.is_whitespace() || PUNCTUATION.contains(character) || arrow)
            && !word.is_empty()
        {
            tokens.push(Token::Word(std::mem::take(&mut word)));
        }
        if arrow {
            characters.next();
            tokens.push(Token::Word("->".to_owned()));
        } else if PUNCTUATION.contains(character) {
            tokens.push(Token::Punct(character));
        } else if !character.is_whitespace() {
            word.push(character);
        }
    }
    if !word.is_empty() {
        tokens.push(Token::Word(word));
    }
}

struct Parser<'a> {
    index: &'a Index,
    tokens: &'a [Token],
    position: usize,
}

impl Parser<'_> {
    /// One type that ends before any of `ends` or the tokens' end.
    fn whole(mut self, ends: &[char]) -> Result<Ty, String> {
        let ty = self.ty()?;
        match self.tokens.get(self.position) {
            None => Ok(ty),
            Some(Token::Punct(end)) if ends.contains(end) => Ok(ty),
            Some(token) => Err(format!("unexpected {token:?} in a type")),
        }
    }

    fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        token
    }

    fn eat(&mut self, punctuation: char) -> bool {
        let found = self.tokens.get(self.position) == Some(&Token::Punct(punctuation));
        if found {
            self.position += 1;
        }
        found
    }

    fn expect(&mut self, punctuation: char) -> Result<(), String> {
        if self.eat(punctuation) {
            Ok(())
        } else {
            Err(format!("expected {punctuation}"))
        }
    }

    fn ty(&mut self) -> Result<Ty, String> {
        let mut ty = self.base()?;
        while self.eat('?') || self.eat('!') {
            ty = ty.optional();
        }
        Ok(ty)
    }

    fn base(&mut self) -> Result<Ty, String> {
        match self.next().cloned() {
            Some(Token::Word(word)) if word == "any" => self.base(),
            Some(Token::Word(word)) if word == "some" => {
                Err("opaque types are not bound".to_owned())
            }
            Some(Token::Word(word)) if word.starts_with('@') || word == "inout" => {
                Err(format!("{word} parameters are not bound"))
            }
            Some(Token::Punct('[')) => self.collection(),
            Some(Token::Punct('(')) if self.eat(')') => self.not_function(Ty::Void),
            Some(Token::Punct('(')) => {
                let ty = self.ty()?;
                if !self.eat(')') {
                    return Err("tuples and closures are not bound".to_owned());
                }
                self.not_function(ty)
            }
            Some(Token::Name { spelling, usr }) => self.named(spelling, usr),
            Some(Token::Word(word)) => self.named(word, None),
            token => Err(format!("unexpected {token:?} in a type")),
        }
    }

    /// `ty`, the parenthesized start of a type, unless a function type continues it.
    fn not_function(&self, ty: Ty) -> Result<Ty, String> {
        match self.tokens.get(self.position) {
            Some(Token::Word(word)) if ["->", "throws", "async"].contains(&word.as_str()) => {
                Err("closures are not bound".to_owned())
            }
            _ => Ok(ty),
        }
    }

    fn collection(&mut self) -> Result<Ty, String> {
        let element = self.ty()?;
        if !self.eat(':') {
            self.expect(']')?;
            return Ok(Ty::List(Box::new(element), Collection::Array));
        }
        let value = self.ty()?;
        self.expect(']')?;
        record(&element, value)
    }

    fn named(&mut self, spelling: String, mut usr: Option<String>) -> Result<Ty, String> {
        let mut path = vec![spelling];
        while self.tokens.get(self.position) == Some(&Token::Punct('.')) {
            let Some(Token::Name {
                spelling,
                usr: next,
            }) = self.tokens.get(self.position + 1).cloned()
            else {
                break;
            };
            self.position += 2;
            path.push(spelling);
            usr = next;
        }
        let mut arguments = Vec::new();
        if self.eat('<') {
            loop {
                arguments.push(self.ty()?);
                if !self.eat(',') {
                    break;
                }
            }
            self.expect('>')?;
        }
        self.resolve(&path, usr.as_deref(), arguments)
    }

    fn resolve(
        &self,
        path: &[String],
        usr: Option<&str>,
        arguments: Vec<Ty>,
    ) -> Result<Ty, String> {
        let symbol = usr
            .and_then(|usr| self.index.symbol(usr))
            .or_else(|| self.index.type_at(path));
        match symbol {
            Some(symbol) => named(self.index, symbol),
            None => builtin(path, arguments),
        }
    }
}

/// A reference to the bound type `symbol`.
pub(super) fn named(index: &Index, symbol: &Symbol) -> Result<Ty, String> {
    let path = symbol.path_components.clone();
    let native = path.join(".");
    let named = |native: String, kind| {
        Ok(Ty::Named(Named {
            path: path.clone(),
            native,
            kind,
        }))
    };
    match symbol.kind() {
        "swift.class" => named(native, NamedKind::Object),
        "swift.protocol" => named(format!("any {native}"), NamedKind::Object),
        "swift.struct" if index.is_option_set(symbol) => named(native, NamedKind::Options),
        "swift.struct" => named(native, NamedKind::Struct),
        "swift.enum" => named(native, NamedKind::Enum),
        "swift.typealias" => {
            let target = alias_target(index, symbol)?;
            named(native, NamedKind::Alias(Box::new(target)))
        }
        kind => Err(format!("{native} is a {kind}")),
    }
}

fn builtin(path: &[String], arguments: Vec<Ty>) -> Result<Ty, String> {
    let name = path.join(".");
    let mut arguments = arguments.into_iter();
    let ty = match name.as_str() {
        "Bool" => Ty::Bool,
        "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt" | "UInt8" | "UInt16" | "UInt32"
        | "UInt64" => Ty::Int(name),
        "Double" | "Float" | "Float32" | "Float64" | "CGFloat" | "TimeInterval" => Ty::Float(name),
        "String" | "URL" | "UUID" | "Locale" | "TimeZone" => Ty::String(name),
        "NSNumber" => Ty::Number(name),
        "Date" => Ty::Date,
        "Data" => Ty::Bytes,
        "Error" => Ty::Error,
        "NSErrorPointer" => Ty::OutError,
        "Void" => Ty::Void,
        "Optional" => arguments
            .next()
            .ok_or("Optional needs its type")?
            .optional(),
        "Array" => Ty::List(
            Box::new(arguments.next().ok_or("Array needs its type")?),
            Collection::Array,
        ),
        "Set" => Ty::List(
            Box::new(arguments.next().ok_or("Set needs its type")?),
            Collection::Set,
        ),
        "Dictionary" => {
            let key = arguments.next().ok_or("Dictionary needs its types")?;
            record(&key, arguments.next().ok_or("Dictionary needs its types")?)?
        }
        _ => return Err(format!("{name} is not bound")),
    };
    Ok(ty)
}

fn record(key: &Ty, value: Ty) -> Result<Ty, String> {
    if *key == Ty::String("String".to_owned()) {
        Ok(Ty::Record(Box::new(value)))
    } else {
        Err("only String keys are bound".to_owned())
    }
}
