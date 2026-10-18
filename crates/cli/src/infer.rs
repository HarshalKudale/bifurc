//! Type inference from JSON samples and the code that states the inferred type in TypeScript, zod, Python or Go.
//! Pure functions, no I/O: `bifurc types` feeds it response bodies, and an OpenAPI inference can reuse it.
//!
//! All samples merge into one [`Ty`]: a field is optional when some object that was seen lacks it, `null` joins a
//! type as a union member, and numbers are integers only while every number seen was integral.

use serde_json::Value;

/// Nesting deeper than this is `unknown` (also keeps a hostile body from recursing without end).
pub const MAX_DEPTH: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Unknown,
    Null,
    Bool,
    Int,
    Num,
    Str,
    Array(Box<Ty>),
    Object(Vec<Field>),
    /// Two or more of the kinds above, in the fixed order string, number, boolean, array, object, null.
    Union(Vec<Ty>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub optional: bool,
}

#[derive(Default)]
struct Node {
    null: bool,
    boolean: bool,
    int: bool,
    num: bool,
    string: bool,
    deep: bool,
    array: Option<Box<Node>>,
    object: Option<Obj>,
}

#[derive(Default)]
struct Obj {
    count: usize,
    /// Name, how many objects carried it, and what it held.
    fields: Vec<(String, usize, Node)>,
}

impl Node {
    fn add(&mut self, value: &Value, depth: usize) {
        match value {
            Value::Null => self.null = true,
            Value::Bool(_) => self.boolean = true,
            Value::String(_) => self.string = true,
            Value::Number(n) if n.as_f64().is_some_and(|f| f.fract() != 0.0) => self.num = true,
            Value::Number(_) => self.int = true,
            _ if depth >= MAX_DEPTH => self.deep = true,
            Value::Array(items) => {
                let element = self.array.get_or_insert_with(Default::default);
                items.iter().for_each(|item| element.add(item, depth + 1));
            }
            Value::Object(map) => {
                let object = self.object.get_or_insert_with(Default::default);
                object.count += 1;
                for (key, item) in map {
                    let at = match object.fields.iter().position(|f| &f.0 == key) {
                        Some(at) => at,
                        None => {
                            object.fields.push((key.clone(), 0, Node::default()));
                            object.fields.len() - 1
                        }
                    };
                    object.fields[at].1 += 1;
                    object.fields[at].2.add(item, depth + 1);
                }
            }
        }
    }

    fn ty(&self) -> Ty {
        if self.deep {
            return Ty::Unknown;
        }
        let mut members = Vec::new();
        if self.string {
            members.push(Ty::Str);
        }
        if self.num {
            members.push(Ty::Num);
        } else if self.int {
            members.push(Ty::Int);
        }
        if self.boolean {
            members.push(Ty::Bool);
        }
        if let Some(element) = &self.array {
            members.push(Ty::Array(Box::new(element.ty())));
        }
        if let Some(object) = &self.object {
            let fields = object.fields.iter().map(|(name, n, node)| Field {
                name: name.clone(),
                ty: node.ty(),
                optional: *n < object.count,
            });
            members.push(Ty::Object(fields.collect()));
        }
        if self.null {
            members.push(Ty::Null);
        }
        match members.len() {
            0 => Ty::Unknown,
            1 => members.remove(0),
            _ => Ty::Union(members),
        }
    }
}

/// The one type that every sample fits. No samples is `unknown`.
pub fn infer(samples: &[Value]) -> Ty {
    let mut root = Node::default();
    samples.iter().for_each(|sample| root.add(sample, 0));
    root.ty()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lang {
    Ts,
    Zod,
    Python,
    Go,
}

impl Lang {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ts" => Some(Self::Ts),
            "zod" => Some(Self::Zod),
            "python" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ts => "ts",
            Self::Zod => "zod",
            Self::Python => "python",
            Self::Go => "go",
        }
    }
}

/// The code declaring `ty` as `name`, under a first comment line `header`.
pub fn emit(lang: Lang, name: &str, ty: &Ty, header: &str) -> String {
    let (comment, body) = match lang {
        Lang::Ts => ("//", format!("export type {name} = {};\n", ts(ty, 0))),
        Lang::Zod => (
            "//",
            format!(
                "import {{ z }} from \"zod\";\n\nexport const {name} = {};\nexport type {name} = z.infer<typeof {name}>;\n",
                zod(ty, 0)
            ),
        ),
        Lang::Python => ("#", python(name, ty)),
        Lang::Go => ("//", go(name, ty)),
    };
    format!("{comment} {header}\n{body}")
}

/// A name in PascalCase: the ASCII letters and digits of `text`, each word capitalised, `T` before a leading digit.
/// Empty when `text` has none.
pub fn pascal(text: &str) -> String {
    let mut out = String::new();
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        out.extend(word.chars().take(1).map(|c| c.to_ascii_uppercase()));
        out.push_str(&word[1..]);
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) { format!("T{out}") } else { out }
}

/// The non-null members of `ty` and whether `null` is one of them.
fn parts(ty: &Ty) -> (Vec<&Ty>, bool) {
    match ty {
        Ty::Union(members) => (members.iter().filter(|m| **m != Ty::Null).collect(), members.contains(&Ty::Null)),
        Ty::Null => (Vec::new(), true),
        other => (vec![other], false),
    }
}

fn pad(depth: usize) -> String {
    "  ".repeat(depth)
}

fn is_ident(key: &str) -> bool {
    key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn quoted(key: &str) -> String {
    Value::String(key.into()).to_string()
}

fn key_of(key: &str) -> String {
    if is_ident(key) { key.into() } else { quoted(key) }
}

fn ts(ty: &Ty, depth: usize) -> String {
    let (members, null) = parts(ty);
    let mut out: Vec<String> = members.iter().map(|m| ts_one(m, depth)).collect();
    if null {
        out.push("null".into());
    }
    out.join(" | ")
}

fn ts_one(ty: &Ty, depth: usize) -> String {
    match ty {
        Ty::Str => "string".into(),
        Ty::Int | Ty::Num => "number".into(),
        Ty::Bool => "boolean".into(),
        Ty::Null => "null".into(),
        Ty::Unknown | Ty::Union(_) => "unknown".into(),
        Ty::Array(element) => {
            let (members, null) = parts(element);
            let inner = ts(element, depth);
            if members.len() + usize::from(null) > 1 { format!("({inner})[]") } else { format!("{inner}[]") }
        }
        Ty::Object(fields) if fields.is_empty() => "Record<string, unknown>".into(),
        Ty::Object(fields) => {
            let mut out = String::from("{\n");
            for f in fields {
                let mark = if f.optional { "?" } else { "" };
                out += &format!("{}{}{mark}: {};\n", pad(depth + 1), key_of(&f.name), ts(&f.ty, depth + 1));
            }
            out + &pad(depth) + "}"
        }
    }
}

fn zod(ty: &Ty, depth: usize) -> String {
    let (members, null) = parts(ty);
    let items: Vec<String> = members.iter().map(|m| zod_one(m, depth)).collect();
    let base = match items.len() {
        0 => return "z.null()".into(),
        1 => items[0].clone(),
        _ => format!("z.union([{}])", items.join(", ")),
    };
    if null { format!("{base}.nullable()") } else { base }
}

fn zod_one(ty: &Ty, depth: usize) -> String {
    match ty {
        Ty::Str => "z.string()".into(),
        Ty::Int => "z.number().int()".into(),
        Ty::Num => "z.number()".into(),
        Ty::Bool => "z.boolean()".into(),
        Ty::Null => "z.null()".into(),
        Ty::Unknown | Ty::Union(_) => "z.unknown()".into(),
        Ty::Array(element) => format!("z.array({})", zod(element, depth)),
        Ty::Object(fields) if fields.is_empty() => "z.record(z.string(), z.unknown())".into(),
        Ty::Object(fields) => {
            let mut out = String::from("z.object({\n");
            for f in fields {
                let mark = if f.optional { ".optional()" } else { "" };
                out += &format!("{}{}: {}{mark},\n", pad(depth + 1), key_of(&f.name), zod(&f.ty, depth + 1));
            }
            out + &pad(depth) + "})"
        }
    }
}

/// Names given to the classes or structs of nested objects: the parent's name plus the field's, `Item` for array
/// elements, a number added when a name is taken.
#[derive(Default)]
struct Names {
    used: Vec<String>,
    defs: Vec<String>,
}

impl Names {
    fn fresh(&mut self, hint: &str) -> String {
        let hint = if hint.is_empty() { "Object" } else { hint };
        let mut name = hint.to_string();
        let mut n = 1;
        while self.used.contains(&name) {
            n += 1;
            name = format!("{hint}{n}");
        }
        self.used.push(name.clone());
        name
    }
}

const PY_KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

#[derive(Default)]
struct Py {
    names: Names,
    /// The `typing` names used, for the import line.
    typing: Vec<&'static str>,
}

impl Py {
    fn need(&mut self, name: &'static str) {
        if !self.typing.contains(&name) {
            self.typing.push(name);
        }
    }

    fn expr(&mut self, ty: &Ty, hint: &str) -> String {
        let (members, null) = parts(ty);
        let mut items: Vec<String> = members.iter().map(|m| self.one(m, hint)).collect();
        let base = match items.len() {
            0 => return "None".into(),
            1 => items.remove(0),
            _ => {
                self.need("Union");
                format!("Union[{}]", items.join(", "))
            }
        };
        if null {
            self.need("Optional");
            format!("Optional[{base}]")
        } else {
            base
        }
    }

    fn one(&mut self, ty: &Ty, hint: &str) -> String {
        match ty {
            Ty::Str => "str".into(),
            Ty::Int => "int".into(),
            Ty::Num => "float".into(),
            Ty::Bool => "bool".into(),
            Ty::Null => "None".into(),
            Ty::Unknown | Ty::Union(_) => {
                self.need("Any");
                "Any".into()
            }
            Ty::Array(element) => format!("list[{}]", self.expr(element, &format!("{hint}Item"))),
            Ty::Object(fields) if fields.is_empty() => {
                self.need("Any");
                "dict[str, Any]".into()
            }
            Ty::Object(fields) => {
                let name = self.names.fresh(hint);
                self.class(&name, fields);
                name
            }
        }
    }

    /// Declare the class of `fields` (after the classes its fields need).
    fn class(&mut self, name: &str, fields: &[Field]) {
        let mut rows = Vec::new();
        for f in fields {
            let mut ty = self.expr(&f.ty, &format!("{name}{}", pascal(&f.name)));
            if f.optional {
                self.need("NotRequired");
                ty = format!("NotRequired[{ty}]");
            }
            rows.push((f.name.as_str(), ty));
        }
        self.need("TypedDict");
        let plain = fields
            .iter()
            .all(|f| is_ident(&f.name) && !f.name.contains('$') && !PY_KEYWORDS.contains(&f.name.as_str()));
        let def = if plain {
            let body: String = rows.iter().map(|(k, t)| format!("    {k}: {t}\n")).collect();
            format!("class {name}(TypedDict):\n{body}")
        } else {
            let body: String = rows.iter().map(|(k, t)| format!("    {}: {t},\n", quoted(k))).collect();
            format!("{name} = TypedDict(\"{name}\", {{\n{body}}})\n")
        };
        self.names.defs.push(def);
    }
}

fn python(name: &str, ty: &Ty) -> String {
    let mut py = Py::default();
    py.names.used.push(name.into());
    match ty {
        Ty::Object(fields) if !fields.is_empty() => py.class(name, fields),
        _ => {
            let expr = py.expr(ty, name);
            py.names.defs.push(format!("{name} = {expr}\n"));
        }
    }
    py.typing.sort_unstable();
    let needs = if py.typing.contains(&"NotRequired") { "# Needs Python 3.11+ (typing.NotRequired).\n" } else { "" };
    let imports = format!("{needs}from typing import {}\n\n", py.typing.join(", "));
    imports + &py.names.defs.join("\n\n")
}

#[derive(Default)]
struct Go {
    names: Names,
}

impl Go {
    fn ty(&mut self, ty: &Ty, hint: &str, optional: bool) -> String {
        let (members, null) = parts(ty);
        let [only] = members.as_slice() else { return "any".into() };
        let text = match only {
            Ty::Str => "string".into(),
            Ty::Int => "int64".into(),
            Ty::Num => "float64".into(),
            Ty::Bool => "bool".into(),
            Ty::Array(element) => format!("[]{}", self.ty(element, &format!("{hint}Item"), false)),
            Ty::Object(fields) if fields.is_empty() => "map[string]any".into(),
            Ty::Object(fields) => {
                let name = self.names.fresh(hint);
                self.strukt(&name, fields);
                name
            }
            _ => "any".into(),
        };
        let nilable = text.starts_with("[]") || text.starts_with("map[") || text == "any";
        if (null || optional) && !nilable { format!("*{text}") } else { text }
    }

    fn strukt(&mut self, name: &str, fields: &[Field]) {
        // The parent is listed before the types it needs.
        let slot = self.names.defs.len();
        self.names.defs.push(String::new());
        let mut taken: Vec<String> = Vec::new();
        let mut body = String::new();
        for f in fields {
            let mut field = pascal(&f.name);
            if field.is_empty() {
                field = "Field".into();
            }
            if field.starts_with(|c: char| c.is_ascii_digit()) {
                field.insert(0, 'F');
            }
            let base = field.clone();
            let mut n = 1;
            while taken.contains(&field) {
                n += 1;
                field = format!("{base}{n}");
            }
            taken.push(field.clone());
            let ty = self.ty(&f.ty, &format!("{name}{base}"), f.optional);
            let omit = if f.optional { ",omitempty" } else { "" };
            body += &format!("\t{field} {ty} `json:\"{}{omit}\"`\n", f.name);
        }
        self.names.defs[slot] = format!("type {name} struct {{\n{body}}}\n");
    }
}

// shortcut: Go fields are not gofmt-aligned and a key with a quote or backtick breaks its tag; run gofmt, upgrade
// by escaping the tag if such keys turn up.
fn go(name: &str, ty: &Ty) -> String {
    let mut go = Go::default();
    go.names.used.push(name.into());
    match ty {
        Ty::Object(fields) if !fields.is_empty() => go.strukt(name, fields),
        _ => {
            let slot = go.names.defs.len();
            go.names.defs.push(String::new());
            let text = go.ty(ty, name, false);
            go.names.defs[slot] = format!("type {name} {text}\n");
        }
    }
    go.names.defs.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field(name: &str, ty: Ty, optional: bool) -> Field {
        Field { name: name.into(), ty, optional }
    }

    #[test]
    fn fields_missing_in_some_samples_are_optional_and_keep_first_order() {
        let ty = infer(&[json!({"b": 1, "a": "x"}), json!({"a": "y", "c": true, "b": 2})]);
        assert_eq!(
            ty,
            Ty::Object(vec![field("b", Ty::Int, false), field("a", Ty::Str, false), field("c", Ty::Bool, true)])
        );
    }

    #[test]
    fn null_joins_the_type_and_null_alone_stays_null() {
        let ty = infer(&[json!({"a": null, "n": null}), json!({"a": "x", "n": null})]);
        let Ty::Object(fields) = ty else { panic!() };
        assert_eq!(fields[0].ty, Ty::Union(vec![Ty::Str, Ty::Null]));
        assert_eq!(fields[1].ty, Ty::Null);
    }

    #[test]
    fn integers_stay_integers_until_a_fraction_shows_up() {
        assert_eq!(infer(&[json!(1), json!(2.0)]), Ty::Int);
        assert_eq!(infer(&[json!(1), json!(2.5)]), Ty::Num);
    }

    #[test]
    fn different_kinds_make_a_union_in_a_fixed_order() {
        let ty = infer(&[json!(1), json!("a"), json!([1]), json!(false)]);
        assert_eq!(ty, Ty::Union(vec![Ty::Str, Ty::Int, Ty::Bool, Ty::Array(Box::new(Ty::Int))]));
    }

    #[test]
    fn arrays_merge_every_element_of_every_sample() {
        let ty = infer(&[json!({"xs": [{"a": 1}, {"a": 2, "b": "x"}]}), json!({"xs": [{"c": null}]})]);
        let Ty::Object(fields) = ty else { panic!() };
        let Ty::Array(element) = &fields[0].ty else { panic!() };
        assert_eq!(
            **element,
            Ty::Object(vec![field("a", Ty::Int, true), field("b", Ty::Str, true), field("c", Ty::Null, true)])
        );
    }

    #[test]
    fn empty_arrays_everywhere_are_unknown_and_roots_may_be_arrays_or_scalars() {
        assert_eq!(infer(&[json!([]), json!([])]), Ty::Array(Box::new(Ty::Unknown)));
        assert_eq!(infer(&[json!([1, 2])]), Ty::Array(Box::new(Ty::Int)));
        assert_eq!(infer(&[json!("x")]), Ty::Str);
        assert_eq!(infer(&[]), Ty::Unknown);
    }

    #[test]
    fn nesting_past_the_cap_is_unknown() {
        let mut value = json!(1);
        for _ in 0..20 {
            value = json!([value]);
        }
        let mut ty = infer(&[value]);
        let mut depth = 0;
        while let Ty::Array(inner) = ty {
            ty = *inner;
            depth += 1;
        }
        assert_eq!((ty, depth), (Ty::Unknown, MAX_DEPTH));
    }

    fn model() -> Ty {
        infer(&[
            json!({"id": 1, "name": "a", "tags": ["x"], "user-id": 5, "items": [{"sku": "s", "qty": 2}]}),
            json!({"id": 2, "name": null, "tags": [], "user-id": 6, "price": 1.5, "items": []}),
        ])
    }

    #[test]
    fn pascal_names() {
        assert_eq!(pascal("Get orders"), "GetOrders");
        assert_eq!(pascal("2fa login!"), "T2faLogin");
        assert_eq!(pascal("user_id"), "UserId");
        assert_eq!(pascal("--"), "");
    }

    #[test]
    fn ts_golden() {
        assert_eq!(
            emit(Lang::Ts, "R", &model(), "Inferred by bifurc from 2 responses of \"X\""),
            r#"// Inferred by bifurc from 2 responses of "X"
export type R = {
  id: number;
  name: string | null;
  tags: string[];
  "user-id": number;
  items: {
    sku: string;
    qty: number;
  }[];
  price?: number;
};
"#
        );
        let union = emit(Lang::Ts, "U", &infer(&[json!([1, "a"])]), "h");
        assert_eq!(union, "// h\nexport type U = (string | number)[];\n");
    }

    #[test]
    fn zod_golden() {
        assert_eq!(
            emit(Lang::Zod, "R", &model(), "h"),
            r#"// h
import { z } from "zod";

export const R = z.object({
  id: z.number().int(),
  name: z.string().nullable(),
  tags: z.array(z.string()),
  "user-id": z.number().int(),
  items: z.array(z.object({
    sku: z.string(),
    qty: z.number().int(),
  })),
  price: z.number().optional(),
});
export type R = z.infer<typeof R>;
"#
        );
    }

    #[test]
    fn python_golden() {
        assert_eq!(
            emit(Lang::Python, "R", &model(), "h"),
            r#"# h
# Needs Python 3.11+ (typing.NotRequired).
from typing import NotRequired, Optional, TypedDict

class RItemsItem(TypedDict):
    sku: str
    qty: int


R = TypedDict("R", {
    "id": int,
    "name": Optional[str],
    "tags": list[str],
    "user-id": int,
    "items": list[RItemsItem],
    "price": NotRequired[float],
})
"#
        );
        assert_eq!(
            emit(Lang::Python, "L", &infer(&[json!([{"a": 1}])]), "h"),
            "# h\nfrom typing import TypedDict\n\nclass LItem(TypedDict):\n    a: int\n\n\nL = list[LItem]\n"
        );
    }

    #[test]
    fn go_golden() {
        assert_eq!(
            emit(Lang::Go, "R", &model(), "h"),
            r#"// h
type R struct {
	Id int64 `json:"id"`
	Name *string `json:"name"`
	Tags []string `json:"tags"`
	UserId int64 `json:"user-id"`
	Items []RItemsItem `json:"items"`
	Price *float64 `json:"price,omitempty"`
}

type RItemsItem struct {
	Sku string `json:"sku"`
	Qty int64 `json:"qty"`
}
"#
        );
        assert_eq!(
            emit(Lang::Go, "L", &infer(&[json!([{"a": 1}])]), "h"),
            "// h\ntype L []LItem\n\ntype LItem struct {\n\tA int64 `json:\"a\"`\n}\n"
        );
        assert_eq!(emit(Lang::Go, "S", &infer(&[json!("x")]), "h"), "// h\ntype S string\n");
    }
}
