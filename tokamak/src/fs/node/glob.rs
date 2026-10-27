//! `node:fs` glob: the walk workerd's glob makes through the filesystem, in its
//! order, matching names with the regular expressions it builds from each
//! pattern.

#![allow(clippy::wildcard_imports)]

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use indexmap::IndexMap;

use super::*;

/// The deepest directory level a glob walks below its working directory.
const MAX_DEPTH: usize = 256;

/// `fs.globSync(pattern, options)`.
pub(super) fn glob_sync<'js>(
    ctx: Ctx<'js>,
    pattern: Value<'js>,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let array = Array::new(ctx.clone())?;
    for (index, value) in glob_values(&ctx, pattern, options.0)?
        .into_iter()
        .enumerate()
    {
        array.set(index, value)?;
    }
    Ok(array.into_value())
}

/// workerd's `fs.promises.glob`, an async generator over `globSync`'s matches.
const GLOB_PROMISE: &str = r"(globSync) => async function* glob(pattern, options = {}) {
  const results = await Promise.resolve(globSync(pattern, options));
  for (const result of results) {
    yield result;
  }
}";

/// `fs.promises.glob(pattern, options)`.
pub(super) fn glob_promise<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    let glob: Function = ctx.eval(GLOB_PROMISE)?;
    glob.call((Function::new(ctx.clone(), glob_sync as ValueOperation)?,))
}

/// The matches of `pattern` under `options`, as names or dirents.
fn glob_values<'js>(
    ctx: &Ctx<'js>,
    pattern: Value<'js>,
    options: Option<Value<'js>>,
) -> rquickjs::Result<Vec<Value<'js>>> {
    let patterns = glob_patterns(ctx, pattern)?;
    let options = GlobOptions::read(ctx, options)?;
    let mut walk = GlobWalk::new(ctx, &options.cwd);
    for pattern in &patterns {
        for expanded in expand_braces(pattern) {
            walk.walk_pattern(&expanded)?;
        }
    }
    options.results(ctx, walk.matches)
}

/// workerd's reading of the `pattern` argument: a string, or the values an
/// iterable spreads to.
const GLOB_PATTERNS: &str = "(pattern) => typeof pattern === 'string' ? [pattern] : [...pattern]";

/// The patterns `pattern` gives, each of which must be a string.
fn glob_patterns<'js>(ctx: &Ctx<'js>, pattern: Value<'js>) -> rquickjs::Result<Vec<String>> {
    let spread: Function = ctx.eval(GLOB_PATTERNS)?;
    let patterns: Array = spread.call((pattern,))?;
    patterns
        .iter::<Value>()
        .map(|value| match value?.into_string() {
            Some(text) => text.to_string(),
            None => Err(throw_type_error(
                ctx,
                "ERR_INVALID_ARG_TYPE",
                "The \"pattern\" argument must be of type string",
            )),
        })
        .collect()
}

/// The options a glob reads, as workerd reads them.
struct GlobOptions<'js> {
    cwd: String,
    with_file_types: bool,
    exclude: Exclude<'js>,
}

/// What a glob leaves out of its matches.
enum Exclude<'js> {
    Nothing,
    /// The matches, as names or dirents, the function returns truthy for.
    Function(Function<'js>),
    /// The names any of these expressions match.
    Patterns(Vec<NameMatcher<'js>>),
}

impl<'js> GlobOptions<'js> {
    fn read(ctx: &Ctx<'js>, options: Option<Value<'js>>) -> rquickjs::Result<Self> {
        let options = match options.filter(|value| !value.is_undefined()) {
            None => Object::new(ctx.clone())?,
            Some(value) if value.is_object() && !value.is_function() => Object::from_value(value)?,
            Some(_) => {
                return Err(throw_type_error(
                    ctx,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"options\" argument must be of type object",
                ));
            }
        };
        Ok(Self {
            cwd: glob_cwd(ctx, options.get("cwd")?)?,
            with_file_types: Coerced::<bool>::from_js(ctx, options.get("withFileTypes")?)?.0,
            exclude: Exclude::read(ctx, options.get("exclude")?)?,
        })
    }

    /// The values for `matches`, each named relative to the working directory,
    /// with the excluded ones left out.
    fn results(
        &self,
        ctx: &Ctx<'js>,
        matches: IndexMap<String, Option<DirectoryEntry>>,
    ) -> rquickjs::Result<Vec<Value<'js>>> {
        let mut values = Vec::new();
        for (relative, entry) in matches {
            if relative.is_empty() || self.exclude.matches_name(&relative)? {
                continue;
            }
            let value = if self.with_file_types {
                self.dirent_at(ctx, &relative, entry.as_ref())?.into_value()
            } else {
                relative.into_js(ctx)?
            };
            if !self.exclude.excludes_value(ctx, value.clone())? {
                values.push(value);
            }
        }
        Ok(values)
    }

    /// The dirent of the match `relative`, whose parent path joins the working
    /// directory as given and the match's parent.
    fn dirent_at(
        &self,
        ctx: &Ctx<'js>,
        relative: &str,
        entry: Option<&DirectoryEntry>,
    ) -> rquickjs::Result<Object<'js>> {
        let (parent_path, name) = match relative.rsplit_once('/') {
            Some((parent, name)) => (format!("{}/{parent}", self.cwd), name),
            None => (self.cwd.clone(), relative),
        };
        let kind = entry.map_or(0, dirent_type);
        dirent(ctx, name.into_js(ctx)?, kind, &parent_path)
    }
}

/// The `cwd` option: a `file:` URL's path, the string given, or the working
/// directory.
fn glob_cwd<'js>(ctx: &Ctx<'js>, cwd: Value<'js>) -> rquickjs::Result<String> {
    let cwd = match url_object(ctx, &cwd)? {
        Some(url) => url.get("pathname")?,
        None if cwd.is_null() || cwd.is_undefined() => {
            working_directory(ctx.clone())?.into_js(ctx)?
        }
        None => cwd,
    };
    match cwd.as_string() {
        Some(text) => text.to_string(),
        None => Err(throw_type_error(
            ctx,
            "ERR_INVALID_ARG_TYPE",
            "The \"options.cwd\" argument must be of type string",
        )),
    }
}

impl<'js> Exclude<'js> {
    /// The `exclude` option: a function, an array of patterns, or nothing for
    /// any other value.
    fn read(ctx: &Ctx<'js>, exclude: Value<'js>) -> rquickjs::Result<Self> {
        if let Some(function) = exclude.as_function() {
            return Ok(Self::Function(function.clone()));
        }
        let Some(patterns) = exclude.as_array() else {
            return Ok(Self::Nothing);
        };
        let string_class: Value = ctx.globals().get("String")?;
        let mut matchers = Vec::new();
        for pattern in patterns.iter::<Value>() {
            let pattern = pattern?;
            let is_text = pattern.is_string()
                || pattern
                    .as_object()
                    .is_some_and(|object| object.is_instance_of(&string_class));
            if !is_text {
                return Err(Exception::throw_type(
                    ctx,
                    "exclude patterns must be strings",
                ));
            }
            let pattern = Coerced::<String>::from_js(ctx, pattern)?.0;
            for expanded in expand_braces(&pattern) {
                matchers.push(NameMatcher::new(ctx, &exclude_source(&expanded))?);
            }
        }
        Ok(Self::Patterns(matchers))
    }

    fn matches_name(&self, relative: &str) -> rquickjs::Result<bool> {
        let Self::Patterns(matchers) = self else {
            return Ok(false);
        };
        for matcher in matchers {
            if matcher.matches(relative)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn excludes_value(&self, ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<bool> {
        let Self::Function(function) = self else {
            return Ok(false);
        };
        let excluded: Value = function.call((value,))?;
        Coerced::<bool>::from_js(ctx, excluded).map(|excluded| excluded.0)
    }
}

/// A JavaScript `RegExp` and its `test` method.
#[derive(Clone)]
struct NameMatcher<'js> {
    regex: Object<'js>,
    test: Function<'js>,
}

impl<'js> NameMatcher<'js> {
    fn new(ctx: &Ctx<'js>, source: &str) -> rquickjs::Result<Self> {
        let class: Constructor = ctx.globals().get("RegExp")?;
        let regex: Object = class.construct((source,))?;
        Ok(Self {
            test: regex.get("test")?,
            regex,
        })
    }

    fn matches(&self, text: &str) -> rquickjs::Result<bool> {
        self.test.call((This(self.regex.clone()), text))
    }
}

/// One segment of a pattern.
#[derive(Clone)]
enum Segment<'js> {
    /// `.`: the current directory.
    Current,
    /// `..`: the parent directory, never above the working directory.
    Parent,
    /// `**`: any number of directories.
    Globstar,
    /// Any other segment: the names it matches.
    Name(NameMatcher<'js>),
}

impl<'js> Segment<'js> {
    fn compile(ctx: &Ctx<'js>, segment: &str) -> rquickjs::Result<Self> {
        Ok(match segment {
            "." => Self::Current,
            ".." => Self::Parent,
            "**" => Self::Globstar,
            _ => {
                let source = segment_source(&segment.chars().collect::<Vec<_>>());
                Self::Name(NameMatcher::new(ctx, &format!("^{source}$"))?)
            }
        })
    }
}

/// A glob's walk: the listings it has read and the matches it has found, each
/// named relative to the working directory, in the order found.
struct GlobWalk<'a, 'js> {
    ctx: &'a Ctx<'js>,
    cwd: &'a str,
    listings: HashMap<String, Rc<[DirectoryEntry]>>,
    matches: IndexMap<String, Option<DirectoryEntry>>,
    segments: Vec<Segment<'js>>,
    globstars: HashSet<(String, usize)>,
}

impl<'a, 'js> GlobWalk<'a, 'js> {
    fn new(ctx: &'a Ctx<'js>, cwd: &'a str) -> Self {
        Self {
            ctx,
            cwd,
            listings: HashMap::new(),
            matches: IndexMap::new(),
            segments: Vec::new(),
            globstars: HashSet::new(),
        }
    }

    /// Walks from the working directory for the brace-free `pattern`.
    fn walk_pattern(&mut self, pattern: &str) -> rquickjs::Result<()> {
        let segments = pattern_segments(pattern);
        if segments.is_empty() {
            return Ok(());
        }
        self.segments = segments
            .iter()
            .map(|segment| Segment::compile(self.ctx, segment))
            .collect::<rquickjs::Result<_>>()?;
        self.globstars.clear();
        let cwd = self.cwd;
        self.walk(0, cwd, "", 0)
    }

    /// Matches the segments from `index` at `absolute`, which is `relative`
    /// below the working directory and `depth` levels down.
    fn walk(
        &mut self,
        index: usize,
        absolute: &str,
        relative: &str,
        depth: usize,
    ) -> rquickjs::Result<()> {
        if depth >= MAX_DEPTH {
            return Ok(());
        }
        match self.segments.get(index).cloned() {
            None => {
                self.match_reached(absolute, relative);
                Ok(())
            }
            Some(Segment::Current) => self.walk(index + 1, absolute, relative, depth),
            Some(Segment::Parent) => self.walk_up(index, absolute, relative, depth),
            Some(Segment::Globstar) => self.walk_globstar(index, absolute, relative, depth),
            Some(Segment::Name(matcher)) => {
                self.walk_matching(index, &matcher, absolute, relative, depth)
            }
        }
    }

    /// Records `relative`, where the segments ran out, with its entry in the
    /// listing of the directory before `absolute`'s last slash.
    fn match_reached(&mut self, absolute: &str, relative: &str) {
        if self.matches.contains_key(relative) {
            return;
        }
        let parent = parent_of(absolute);
        let name = relative.rsplit('/').next().unwrap_or_default();
        let entry = self
            .listing(parent)
            .iter()
            .find(|entry| entry.name == name)
            .cloned();
        self.matches.insert(relative.to_owned(), entry);
    }

    fn walk_up(
        &mut self,
        index: usize,
        absolute: &str,
        relative: &str,
        depth: usize,
    ) -> rquickjs::Result<()> {
        let below_cwd = absolute
            .strip_prefix(self.cwd)
            .is_some_and(|rest| rest.starts_with('/'));
        if !below_cwd {
            return Ok(());
        }
        let parent = parent_of(absolute);
        let mut parts: Vec<&str> = relative
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        parts.pop();
        self.walk(index + 1, parent, &parts.join("/"), depth)
    }

    fn walk_globstar(
        &mut self,
        index: usize,
        absolute: &str,
        relative: &str,
        depth: usize,
    ) -> rquickjs::Result<()> {
        if !self.globstars.insert((absolute.to_owned(), index)) {
            return Ok(());
        }
        self.walk(index + 1, absolute, relative, depth)?;
        for entry in self.listing(absolute).iter() {
            let (child, child_relative) = child_paths(absolute, relative, &entry.name);
            self.walk(index + 1, &child, &child_relative, depth + 1)?;
            if entry.kind == NodeType::Directory {
                self.walk(index, &child, &child_relative, depth + 1)?;
            }
        }
        Ok(())
    }

    fn walk_matching(
        &mut self,
        index: usize,
        matcher: &NameMatcher<'js>,
        absolute: &str,
        relative: &str,
        depth: usize,
    ) -> rquickjs::Result<()> {
        let last = index + 1 >= self.segments.len();
        for entry in self.listing(absolute).iter() {
            if !matcher.matches(&entry.name)? {
                continue;
            }
            let (child, child_relative) = child_paths(absolute, relative, &entry.name);
            if last {
                self.matches
                    .entry(child_relative)
                    .or_insert_with(|| Some(entry.clone()));
            } else {
                self.walk(index + 1, &child, &child_relative, depth + 1)?;
            }
        }
        Ok(())
    }

    /// The entries of the directory at `path`, none when it cannot be listed.
    fn listing(&mut self, path: &str) -> Rc<[DirectoryEntry]> {
        if let Some(entries) = self.listings.get(path) {
            return Rc::clone(entries);
        }
        let entries: Rc<[DirectoryEntry]> = read_listing(self.ctx, path)
            .unwrap_or_else(|_| {
                let _discarded = self.ctx.catch();
                Vec::new()
            })
            .into();
        self.listings.insert(path.to_owned(), Rc::clone(&entries));
        entries
    }
}

fn read_listing(ctx: &Ctx<'_>, directory: &str) -> rquickjs::Result<Vec<DirectoryEntry>> {
    let directory = path(ctx, directory.into_js(ctx)?)?;
    vfs_call(ctx, |vfs| vfs.read_dir(&directory))
}

/// The absolute and working-directory-relative paths of the child `name`.
fn child_paths(absolute: &str, relative: &str, name: &str) -> (String, String) {
    let child_relative = if relative.is_empty() {
        name.to_owned()
    } else {
        format!("{relative}/{name}")
    };
    (format!("{absolute}/{name}"), child_relative)
}

/// `pattern` with its first top-level brace group expanded, each alternative
/// expanded in turn. A group without a top-level comma stays literal.
fn expand_braces(pattern: &str) -> Vec<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut depth = 0_i32;
    let mut start = None;
    let mut index = 0;
    while let Some(&c) = chars.get(index) {
        match c {
            '\\' if index + 1 < chars.len() => index += 1,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0
                    && let Some(start) = start
                {
                    return expand_group(&chars, start, index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    vec![pattern.to_owned()]
}

/// `chars` with the brace group from `start` to `end` expanded.
fn expand_group(chars: &[char], start: usize, end: usize) -> Vec<String> {
    let prefix: String = chars[..start].iter().collect();
    let body: String = chars[start + 1..end].iter().collect();
    let suffix: String = chars[end + 1..].iter().collect();
    let alternatives = split_top_level(&body, ',');
    if alternatives.len() == 1 {
        return vec![chars.iter().collect()];
    }
    alternatives
        .iter()
        .flat_map(|alternative| expand_braces(&format!("{prefix}{alternative}{suffix}")))
        .collect()
}

/// `text` split at each `separator` outside braces and parentheses, keeping
/// escaped characters.
fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0_i32;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                current.push(c);
                current.extend(chars.next());
            }
            '{' | '(' => {
                depth += 1;
                current.push(c);
            }
            '}' | ')' => {
                depth -= 1;
                current.push(c);
            }
            _ if c == separator && depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// `pattern` without a leading `./`, repeated slashes or a trailing slash.
fn normalized_pattern(pattern: &str) -> String {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let mut normalized = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        if !(c == '/' && normalized.ends_with('/')) {
            normalized.push(c);
        }
    }
    if normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

/// The segments of `pattern`, with each run of `**` segments as one.
fn pattern_segments(pattern: &str) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    for segment in normalized_pattern(pattern).split('/') {
        let repeated_globstar = segment == "**" && segments.last().is_some_and(|last| last == "**");
        if !segment.is_empty() && !repeated_globstar {
            segments.push(segment.to_owned());
        }
    }
    segments
}

/// The regular expression source for the glob segment `segment`.
fn segment_source(segment: &[char]) -> String {
    let mut source = String::new();
    let mut in_class = false;
    let mut index = 0;
    while let Some(&c) = segment.get(index) {
        index += if c == '\\' && index + 1 < segment.len() {
            source.push('\\');
            push_escaped(&mut source, segment[index + 1]);
            2
        } else if in_class {
            in_class = c != ']';
            source.push(c);
            1
        } else {
            push_token(&mut source, segment, index, &mut in_class)
        };
    }
    source
}

/// Appends the source for the token at `index` outside a character class,
/// returning its length.
fn push_token(source: &mut String, segment: &[char], index: usize, in_class: &mut bool) -> usize {
    let c = segment[index];
    if let Some(end) = extglob_end(segment, index) {
        source.push_str(&extglob_source(c, &segment[index + 2..end]));
        return end + 1 - index;
    }
    match c {
        '[' if segment.get(index + 1) == Some(&'!') => {
            *in_class = true;
            source.push_str("[^");
            return 2;
        }
        '[' => {
            *in_class = true;
            source.push('[');
        }
        '*' => source.push_str("[^/]*"),
        '?' => source.push_str("[^/]"),
        '.' | '+' | '^' | '$' | '|' | '(' | ')' | '{' | '}' => {
            source.push('\\');
            source.push(c);
        }
        _ => source.push(c),
    }
    1
}

/// The index of the `)` closing the extglob at `index`, such as `@(a|b)`.
fn extglob_end(segment: &[char], index: usize) -> Option<usize> {
    let starts_extglob = matches!(segment[index], '@' | '*' | '+' | '?' | '!')
        && segment.get(index + 1) == Some(&'(');
    if !starts_extglob {
        return None;
    }
    let mut depth = 0;
    let mut position = index + 1;
    while let Some(&c) = segment.get(position) {
        match c {
            '\\' => position += 1,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(position);
                }
            }
            _ => {}
        }
        position += 1;
    }
    None
}

/// The source for the extglob `kind(inner)`.
fn extglob_source(kind: char, inner: &[char]) -> String {
    let inner: String = inner.iter().collect();
    let group = split_top_level(&inner, '|')
        .iter()
        .map(|alternative| segment_source(&alternative.chars().collect::<Vec<_>>()))
        .collect::<Vec<_>>()
        .join("|");
    match kind {
        '@' => format!("(?:{group})"),
        '*' => format!("(?:{group})*"),
        '+' => format!("(?:{group})+"),
        '?' => format!("(?:{group})?"),
        _ => format!("(?!(?:{group})$)[^/]*"),
    }
}

fn push_escaped(source: &mut String, c: char) {
    if ".+*?^$|()[]{}\\".contains(c) {
        source.push('\\');
    }
    source.push(c);
}

/// The regular expression source matching the relative paths the whole
/// pattern `pattern` names, for `exclude`.
fn exclude_source(pattern: &str) -> String {
    let mut parts: Vec<Option<String>> = Vec::new();
    for segment in normalized_pattern(pattern).split('/') {
        match segment {
            "" | "." => {}
            "**" => parts.push(None),
            ".." => {
                parts.pop();
            }
            _ => parts.push(Some(segment_source(&segment.chars().collect::<Vec<_>>()))),
        }
    }
    let mut source = String::new();
    for (index, part) in parts.iter().enumerate() {
        match part {
            None if parts.len() == 1 => source.push_str(".*"),
            None if index == 0 => source.push_str("(?:.*\\/)?"),
            None if index == parts.len() - 1 => source.push_str("(?:\\/.*)?"),
            None => source.push_str("(?:\\/[^/]+)*(?:\\/)?"),
            Some(part) => {
                if index > 0 && parts[index - 1].is_some() {
                    source.push_str("\\/");
                }
                source.push_str(part);
            }
        }
    }
    format!("^{source}$")
}
