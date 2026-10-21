//! Declarations from the Android SDK's Java stub sources.

/// A class, interface or enum and the classes nested in it.
#[derive(Debug, Default)]
pub(super) struct Class {
    /// Its name and the names of the classes it is nested in, outermost first.
    pub(super) path: Vec<String>,
    pub(super) kind: ClassKind,
    pub(super) modifiers: Vec<String>,
    pub(super) annotations: Vec<String>,
    /// Its type parameters' names.
    pub(super) parameters: Vec<String>,
    pub(super) superclass: Option<JavaType>,
    pub(super) interfaces: Vec<JavaType>,
    pub(super) members: Vec<JavaMember>,
    /// An enum's constants.
    pub(super) constants: Vec<String>,
    pub(super) nested: Vec<Class>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClassKind {
    #[default]
    Class,
    Interface,
    Enum,
    Annotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MemberKind {
    Constructor,
    Method,
    Field,
}

#[derive(Debug)]
pub(super) struct JavaMember {
    pub(super) kind: MemberKind,
    pub(super) name: String,
    pub(super) modifiers: Vec<String>,
    pub(super) annotations: Vec<String>,
    /// A generic method's type parameters' names.
    pub(super) parameters: Vec<String>,
    /// The method's result or the field's type; void for a constructor.
    pub(super) ty: JavaType,
    pub(super) params: Vec<JavaParam>,
    /// A constant field's value, as Java source.
    pub(super) value: Option<String>,
}

#[derive(Debug)]
pub(super) struct JavaParam {
    pub(super) name: String,
    pub(super) ty: JavaType,
    pub(super) annotations: Vec<String>,
    pub(super) varargs: bool,
}

/// A Java type: a qualified name with type arguments, and its array dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct JavaType {
    pub(super) name: String,
    pub(super) arguments: Vec<JavaType>,
    pub(super) dimensions: usize,
}

impl JavaType {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// The JVM descriptor of the erased type, with `erase` naming each type
    /// variable's erasure.
    pub(super) fn descriptor(&self, erase: &dyn Fn(&str) -> Option<String>) -> String {
        let element = match self.name.as_str() {
            "void" => "V".to_owned(),
            "boolean" => "Z".to_owned(),
            "byte" => "B".to_owned(),
            "char" => "C".to_owned(),
            "short" => "S".to_owned(),
            "int" => "I".to_owned(),
            "long" => "J".to_owned(),
            "float" => "F".to_owned(),
            "double" => "D".to_owned(),
            name => format!(
                "L{};",
                erase(name)
                    .unwrap_or_else(|| name.to_owned())
                    .replace('.', "/")
            ),
        };
        "[".repeat(self.dimensions) + &element
    }
}

/// A stub source file's package and the classes it declares.
#[derive(Debug, Default)]
pub(super) struct File {
    pub(super) package: String,
    pub(super) classes: Vec<Class>,
}

pub(super) fn parse(source: &str) -> Result<File, String> {
    let tokens = tokenize(source)?;
    Parser {
        tokens: &tokens,
        position: 0,
    }
    .file()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Punct(char),
    /// A string, character or number literal, as written.
    Literal(String),
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let characters: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut position = 0;
    while let Some(&character) = characters.get(position) {
        let next = characters.get(position + 1).copied();
        position = match (character, next) {
            (character, _) if character.is_whitespace() => position + 1,
            ('/', Some('/')) => skip_until(&characters, position, "\n"),
            ('/', Some('*')) => skip_until(&characters, position + 2, "*/"),
            ('"' | '\'', _) => {
                let end = literal_end(&characters, position)?;
                tokens.push(Token::Literal(characters[position..end].iter().collect()));
                end
            }
            (character, _)
                if character.is_ascii_digit()
                    || (character == '-' && next.is_some_and(|next| next.is_ascii_digit())) =>
            {
                let end = word_end(&characters, position + 1);
                tokens.push(Token::Literal(characters[position..end].iter().collect()));
                end
            }
            (character, _) if is_word(character) => {
                let end = word_end(&characters, position);
                tokens.push(Token::Word(characters[position..end].iter().collect()));
                end
            }
            (character, _) => {
                tokens.push(Token::Punct(character));
                position + 1
            }
        };
    }
    Ok(tokens)
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

fn word_end(characters: &[char], start: usize) -> usize {
    let mut end = start;
    while characters
        .get(end)
        .is_some_and(|&character| is_word(character) || character == '.')
    {
        end += 1;
    }
    // A qualified name ends before a trailing dot, such as the first of `...`.
    while end > start && characters[end - 1] == '.' {
        end -= 1;
    }
    end
}

fn skip_until(characters: &[char], start: usize, end: &str) -> usize {
    let end: Vec<char> = end.chars().collect();
    let mut position = start;
    while position < characters.len() && !characters[position..].starts_with(&end) {
        position += 1;
    }
    position + end.len()
}

fn literal_end(characters: &[char], start: usize) -> Result<usize, String> {
    let quote = characters[start];
    let mut position = start + 1;
    while let Some(&character) = characters.get(position) {
        match character {
            '\\' => position += 2,
            character if character == quote => return Ok(position + 1),
            _ => position += 1,
        }
    }
    Err("an unterminated literal".to_owned())
}

const MODIFIERS: [&str; 12] = [
    "public",
    "protected",
    "private",
    "static",
    "final",
    "abstract",
    "synchronized",
    "native",
    "default",
    "transient",
    "volatile",
    "strictfp",
];

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn is(&self, punctuation: char) -> bool {
        self.peek() == Some(&Token::Punct(punctuation))
    }

    fn eat(&mut self, punctuation: char) -> bool {
        let found = self.is(punctuation);
        if found {
            self.position += 1;
        }
        found
    }

    fn expect(&mut self, punctuation: char) -> Result<(), String> {
        if self.eat(punctuation) {
            Ok(())
        } else {
            Err(format!("expected {punctuation} at {:?}", self.peek()))
        }
    }

    fn word(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token::Word(word)) => Ok(word),
            token => Err(format!("expected a name, found {token:?}")),
        }
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let found = matches!(self.peek(), Some(Token::Word(found)) if found == word);
        if found {
            self.position += 1;
        }
        found
    }

    fn file(&mut self) -> Result<File, String> {
        let mut file = File::default();
        while self.peek().is_some() {
            if self.eat_word("package") {
                file.package = self.word()?;
                self.expect(';')?;
            } else if self.eat_word("import") {
                self.skip_past(';');
            } else {
                file.classes.push(self.class(&[])?);
            }
        }
        Ok(file)
    }

    fn skip_past(&mut self, punctuation: char) {
        while let Some(token) = self.next() {
            if token == Token::Punct(punctuation) {
                return;
            }
        }
    }

    /// Annotations' names and modifiers, before a declaration.
    fn prefix(&mut self) -> Result<(Vec<String>, Vec<String>), String> {
        let mut annotations = Vec::new();
        let mut modifiers = Vec::new();
        loop {
            if self.is('@')
                && !matches!(self.tokens.get(self.position + 1), Some(Token::Word(word)) if word == "interface")
            {
                self.position += 1;
                annotations.push(self.word()?);
                if self.is('(') {
                    self.skip_balanced('(', ')');
                }
            } else if let Some(Token::Word(word)) = self.peek().filter(
                |token| matches!(token, Token::Word(word) if MODIFIERS.contains(&word.as_str())),
            ) {
                modifiers.push(word.clone());
                self.position += 1;
            } else {
                return Ok((annotations, modifiers));
            }
        }
    }

    fn skip_balanced(&mut self, open: char, close: char) {
        let mut depth = 0;
        while let Some(token) = self.next() {
            if token == Token::Punct(open) {
                depth += 1;
            } else if token == Token::Punct(close) {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    fn class(&mut self, outer: &[String]) -> Result<Class, String> {
        let (annotations, modifiers) = self.prefix()?;
        let kind = if self.eat('@') {
            self.word()?;
            ClassKind::Annotation
        } else {
            match self.word()?.as_str() {
                "class" => ClassKind::Class,
                "interface" => ClassKind::Interface,
                "enum" => ClassKind::Enum,
                word => return Err(format!("expected a class, found {word}")),
            }
        };
        let mut path = outer.to_vec();
        path.push(self.word()?);
        let parameters = self.type_parameters()?;
        let mut class = Class {
            path,
            kind,
            modifiers,
            annotations,
            parameters,
            ..Class::default()
        };
        self.supertypes(&mut class)?;
        self.expect('{')?;
        if kind == ClassKind::Enum {
            self.enum_constants(&mut class)?;
        }
        self.body(&mut class)?;
        Ok(class)
    }

    fn supertypes(&mut self, class: &mut Class) -> Result<(), String> {
        loop {
            if self.eat_word("extends") {
                let types = self.type_list()?;
                if class.kind == ClassKind::Interface {
                    class.interfaces.extend(types);
                } else {
                    class.superclass = types.into_iter().next();
                }
            } else if self.eat_word("implements") {
                class.interfaces.extend(self.type_list()?);
            } else {
                return Ok(());
            }
        }
    }

    fn type_list(&mut self) -> Result<Vec<JavaType>, String> {
        let mut types = vec![self.ty()?];
        while self.eat(',') {
            types.push(self.ty()?);
        }
        Ok(types)
    }

    fn enum_constants(&mut self, class: &mut Class) -> Result<(), String> {
        while let Some(Token::Word(_)) = self.peek() {
            let (_, _) = self.prefix()?;
            class.constants.push(self.word()?);
            if self.is('(') {
                self.skip_balanced('(', ')');
            }
            if self.is('{') {
                self.skip_balanced('{', '}');
            }
            if !self.eat(',') {
                break;
            }
        }
        self.eat(';');
        Ok(())
    }

    fn body(&mut self, class: &mut Class) -> Result<(), String> {
        loop {
            if self.eat('}') {
                return Ok(());
            }
            if self.peek().is_none() {
                return Err(format!("{} does not end", class.path.join(".")));
            }
            if self.declares_class() {
                class.nested.push(self.class(&class.path)?);
            } else if self.initializer() {
                self.skip_balanced('{', '}');
            } else {
                class.members.push(self.member(class)?);
            }
        }
    }

    /// Whether an initializer block follows, moving to its `{`.
    fn initializer(&mut self) -> bool {
        let start = self.position;
        self.eat_word("static");
        let initializer = self.is('{');
        if !initializer {
            self.position = start;
        }
        initializer
    }

    /// Whether a class declaration follows, after any annotations and modifiers.
    fn declares_class(&mut self) -> bool {
        let start = self.position;
        let declares = self.prefix().is_ok()
            && (matches!(self.peek(), Some(Token::Word(word)) if ["class", "interface", "enum"].contains(&word.as_str()))
                || (self.is('@')
                    && matches!(self.tokens.get(self.position + 1), Some(Token::Word(word)) if word == "interface")));
        self.position = start;
        declares
    }

    fn member(&mut self, class: &Class) -> Result<JavaMember, String> {
        let (annotations, modifiers) = self.prefix()?;
        let parameters = self.type_parameters()?;
        let constructor = matches!(self.peek(), Some(Token::Word(word)) if Some(word) == class.path.last())
            && self.tokens.get(self.position + 1) == Some(&Token::Punct('('));
        let ty = if constructor {
            JavaType::named("void")
        } else {
            self.ty()?
        };
        let name = self.word()?;
        let mut member = JavaMember {
            kind: if constructor {
                MemberKind::Constructor
            } else {
                MemberKind::Method
            },
            name,
            modifiers,
            annotations,
            parameters,
            ty,
            params: Vec::new(),
            value: None,
        };
        if self.eat('(') {
            member.params = self.params()?;
            self.method_end();
        } else {
            member.kind = MemberKind::Field;
            member.value = self.field_end();
        }
        Ok(member)
    }

    /// Skips a method's `throws` clause, default value and body.
    fn method_end(&mut self) {
        while let Some(token) = self.peek() {
            match token {
                Token::Punct(';') => {
                    self.position += 1;
                    return;
                }
                Token::Punct('{') => {
                    self.skip_balanced('{', '}');
                    return;
                }
                _ => self.position += 1,
            }
        }
    }

    /// A field's value when it is a single literal, skipping to its end.
    fn field_end(&mut self) -> Option<String> {
        let mut value = None;
        if self.eat('=') {
            let start = self.position;
            self.skip_past(';');
            if let [Token::Literal(literal), Token::Punct(';')] = &self.tokens[start..self.position]
            {
                value = Some(literal.clone());
            } else if let [Token::Word(word), Token::Punct(';')] =
                &self.tokens[start..self.position]
                && (word == "true" || word == "false")
            {
                value = Some(word.clone());
            }
        } else {
            self.skip_past(';');
        }
        value
    }

    fn params(&mut self) -> Result<Vec<JavaParam>, String> {
        let mut params = Vec::new();
        if self.eat(')') {
            return Ok(params);
        }
        loop {
            let (annotations, _) = self.prefix()?;
            let ty = self.ty()?;
            let varargs = self.is('.');
            if varargs {
                self.position += 3;
            }
            let name = self.word()?;
            params.push(JavaParam {
                name,
                ty,
                annotations,
                varargs,
            });
            if !self.eat(',') {
                self.expect(')')?;
                return Ok(params);
            }
        }
    }

    fn type_parameters(&mut self) -> Result<Vec<String>, String> {
        let mut parameters = Vec::new();
        if !self.eat('<') {
            return Ok(parameters);
        }
        loop {
            parameters.push(self.word()?);
            if self.eat_word("extends") {
                self.ty()?;
                while self.eat('&') {
                    self.ty()?;
                }
            }
            if !self.eat(',') {
                self.expect('>')?;
                return Ok(parameters);
            }
        }
    }

    fn ty(&mut self) -> Result<JavaType, String> {
        while self.is('@') {
            self.position += 1;
            self.word()?;
            if self.is('(') {
                self.skip_balanced('(', ')');
            }
        }
        if self.eat('?') {
            let bound = if self.eat_word("extends") {
                self.ty()?
            } else {
                if self.eat_word("super") {
                    self.ty()?;
                }
                JavaType::named("java.lang.Object")
            };
            return Ok(bound);
        }
        let mut ty = JavaType::named(&self.word()?);
        if self.eat('<') {
            loop {
                ty.arguments.push(self.ty()?);
                if !self.eat(',') {
                    break;
                }
            }
            self.expect('>')?;
        }
        while self.is('[') && self.tokens.get(self.position + 1) == Some(&Token::Punct(']')) {
            self.position += 2;
            ty.dimensions += 1;
        }
        Ok(ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STUB: &str = r#"/*
 * Copyright
 */
package android.location;
import java.util.List;
/**
 * Docs with { braces } and "quotes".
 */
@SuppressWarnings({"unchecked", "deprecation", "all"})
public class LocationManager {
LocationManager() { throw new RuntimeException("Stub!"); }
@android.annotation.Nullable
public android.location.Location getLastKnownLocation(@android.annotation.NonNull java.lang.String provider) { throw new RuntimeException("Stub!"); }
public <T> void sendAll(@android.annotation.NonNull java.util.List<T> items, int... codes) throws java.io.IOException { throw new RuntimeException("Stub!"); }
public static final java.lang.String GPS_PROVIDER = "gps";
public static final int CODE = 5; // 0x5
static { CODE = 5; }
@android.annotation.NonNull public static final android.os.Parcelable.Creator<android.location.Location> CREATOR;
public abstract static class Callback {
public Callback() { throw new RuntimeException("Stub!"); }
public void onStarted() { throw new RuntimeException("Stub!"); }
}
}
"#;

    #[test]
    fn parses_classes_members_and_nested_classes() -> Result<(), String> {
        let file = parse(STUB)?;
        assert_eq!(file.package, "android.location");
        let [class] = file.classes.as_slice() else {
            return Err(format!("{:?}", file.classes));
        };
        assert_eq!(class.path, ["LocationManager"]);
        let names: Vec<&str> = class
            .members
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "LocationManager",
                "getLastKnownLocation",
                "sendAll",
                "GPS_PROVIDER",
                "CODE",
                "CREATOR"
            ]
        );
        let location = &class.members[1];
        assert_eq!(location.annotations, ["android.annotation.Nullable"]);
        assert_eq!(
            location.params[0].annotations,
            ["android.annotation.NonNull"]
        );
        let send = &class.members[2];
        assert_eq!(send.parameters, ["T"]);
        assert_eq!(send.params[0].ty.arguments, [JavaType::named("T")]);
        assert!(send.params[1].varargs);
        assert_eq!(class.members[3].value.as_deref(), Some("\"gps\""));
        assert_eq!(class.members[4].value.as_deref(), Some("5"));
        assert_eq!(class.nested[0].path, ["LocationManager", "Callback"]);
        assert_eq!(class.nested[0].members.len(), 2);
        Ok(())
    }

    #[test]
    fn erases_types_in_descriptors() {
        let ty = JavaType {
            name: "T".to_owned(),
            arguments: Vec::new(),
            dimensions: 1,
        };
        let erase = |name: &str| (name == "T").then(|| "java.lang.Object".to_owned());
        assert_eq!(ty.descriptor(&erase), "[Ljava/lang/Object;");
        assert_eq!(JavaType::named("long").descriptor(&erase), "J");
    }
}
