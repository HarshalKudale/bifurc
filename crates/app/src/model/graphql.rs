//! A GraphQL schema as the explorer and the query editor's completion use it.
//!
//! `graphql.introspect` hands back the endpoint's raw answer to the introspection query. [`Schema::parse`]
//! reads that into types with their fields, and [`Schema::completions`] says which field names are valid at a
//! cursor in a query document. Nothing here knows about a window.

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    pub name: String,
    /// The type as written in SDL, such as `[ID!]!`.
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub description: String,
    pub args: Vec<Argument>,
    pub ty: String,
    /// The type with its list and non-null wrappers removed: what a selection inside this field is on.
    pub target: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeInfo {
    pub name: String,
    /// `OBJECT`, `INTERFACE`, `ENUM`, `INPUT_OBJECT`, `UNION` or `SCALAR`.
    pub kind: String,
    pub description: String,
    /// Fields of an object or interface; the input fields of an input object.
    pub fields: Vec<Field>,
    pub enum_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Schema {
    pub query: Option<String>,
    pub mutation: Option<String>,
    pub subscription: Option<String>,
    /// By name. Types the server reserves for itself (`__Schema`, ...) are left out.
    pub types: BTreeMap<String, TypeInfo>,
}

/// One suggestion for the query editor.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub label: String,
    pub detail: String,
    pub description: String,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// A type reference as SDL: `[User!]!`.
fn type_string(ty: &Value) -> String {
    let inner = || ty.get("ofType").map(type_string).unwrap_or_default();
    match ty.get("kind").and_then(Value::as_str) {
        Some("NON_NULL") => format!("{}!", inner()),
        Some("LIST") => format!("[{}]", inner()),
        _ => text(ty, "name"),
    }
}

/// The named type at the bottom of a type reference.
fn target_name(ty: &Value) -> String {
    match ty.get("name").and_then(Value::as_str) {
        Some(name) => name.to_owned(),
        None => ty.get("ofType").map(target_name).unwrap_or_default(),
    }
}

fn parse_field(value: &Value) -> Field {
    let ty = value.get("type").cloned().unwrap_or(Value::Null);
    Field {
        name: text(value, "name"),
        description: text(value, "description"),
        args: value
            .get("args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|arg| Argument { name: text(arg, "name"), ty: arg.get("type").map(type_string).unwrap_or_default() })
            .collect(),
        ty: type_string(&ty),
        target: target_name(&ty),
    }
}

fn root_name(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.get("name")?.as_str().map(str::to_owned)
}

impl Schema {
    /// Reads the body of a response to the introspection query.
    pub fn parse(body: &str) -> Result<Schema, String> {
        let value: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
        let Some(schema) = value.get("data").and_then(|data| data.get("__schema")).filter(|s| s.is_object()) else {
            let reason = value
                .get("errors")
                .and_then(|errors| errors.get(0))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str);
            return Err(reason.unwrap_or("The response holds no schema.").to_owned());
        };
        let types = schema
            .get("types")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|ty| !text(ty, "name").starts_with("__"))
            .map(|ty| {
                let fields = ty.get("fields").or_else(|| ty.get("inputFields"));
                let info = TypeInfo {
                    name: text(ty, "name"),
                    kind: text(ty, "kind"),
                    description: text(ty, "description"),
                    fields: fields.and_then(Value::as_array).into_iter().flatten().map(parse_field).collect(),
                    enum_values: ty
                        .get("enumValues")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|v| text(v, "name"))
                        .collect(),
                };
                (info.name.clone(), info)
            })
            .collect();
        Ok(Schema {
            query: root_name(schema, "queryType"),
            mutation: root_name(schema, "mutationType"),
            subscription: root_name(schema, "subscriptionType"),
            types,
        })
    }

    /// The root operation types, with the keyword that opens each.
    pub fn roots(&self) -> [(&'static str, Option<&TypeInfo>); 3] {
        let find = |name: &Option<String>| name.as_ref().and_then(|name| self.types.get(name));
        [("query", find(&self.query)), ("mutation", find(&self.mutation)), ("subscription", find(&self.subscription))]
    }

    /// The fields a selection set on `type_name` may name. A union has none of its own.
    fn selectable(&self, type_name: &str) -> &[Field] {
        match self.types.get(type_name) {
            Some(info) if matches!(info.kind.as_str(), "OBJECT" | "INTERFACE") => &info.fields,
            _ => &[],
        }
    }

    /// The field names valid at byte `offset` of `document`, narrowed to those starting with the word being
    /// typed. The selection set the cursor is in is found by scanning the text before it; fragment and
    /// inline-fragment type conditions are followed.
    ///
    /// shortcut: nothing is suggested inside an argument list or a string, and a field named `on` is read as a
    /// type condition; upgrade by parsing the document properly.
    pub fn completions(&self, document: &str, offset: usize) -> Vec<Candidate> {
        let offset = offset.min(document.len());
        if !document.is_char_boundary(offset) {
            return Vec::new();
        }
        let before = &document[..offset];
        let word_start = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |ix| ix + 1);
        let prefix = before[word_start..].to_ascii_lowercase();
        let Some(scope) = self.scope_at(&before[..word_start]) else { return Vec::new() };
        let mut found: Vec<Candidate> = self
            .selectable(&scope)
            .iter()
            .filter(|field| field.name.to_ascii_lowercase().starts_with(&prefix))
            .map(|field| Candidate {
                label: field.name.clone(),
                detail: field.ty.clone(),
                description: field.description.clone(),
            })
            .collect();
        if self.types.get(&scope).is_some_and(|info| matches!(info.kind.as_str(), "OBJECT" | "INTERFACE" | "UNION"))
            && "__typename".starts_with(&prefix)
        {
            found.push(Candidate { label: "__typename".into(), detail: "String!".into(), description: String::new() });
        }
        found
    }

    /// The type of the innermost open selection set at the end of `text`. `None` when the end is not in
    /// one (outside any braces, inside an argument list or a string), or when its type is not in the schema.
    fn scope_at(&self, text: &str) -> Option<String> {
        let mut stack: Vec<Option<String>> = Vec::new();
        let mut operation: Option<&str> = None;
        let mut last_name: Option<String> = None;
        let mut previous_was_on = false;
        let mut condition: Option<String> = None;
        let mut skip_name = false;
        let mut parens = 0usize;
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '#' => {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                '"' => {
                    let block = chars[i..].starts_with(&['"', '"', '"']);
                    i += if block { 3 } else { 1 };
                    loop {
                        if i >= chars.len() {
                            return None; // the cursor is inside the string
                        }
                        if block && chars[i..].starts_with(&['"', '"', '"']) {
                            i += 3;
                            break;
                        }
                        if !block && chars[i] == '"' {
                            i += 1;
                            break;
                        }
                        i += if chars[i] == '\\' { 2 } else { 1 };
                    }
                    continue;
                }
                '(' => parens += 1,
                ')' => parens = parens.saturating_sub(1),
                _ if parens > 0 => {}
                '@' => skip_name = true,
                '{' => {
                    let scope = match stack.last() {
                        _ if condition.is_some() => condition.take(),
                        None => match operation.unwrap_or("query") {
                            "mutation" => self.mutation.clone(),
                            "subscription" => self.subscription.clone(),
                            _ => self.query.clone(),
                        },
                        Some(parent) => match (&last_name, parent) {
                            (Some(field), Some(parent)) => self
                                .selectable(parent)
                                .iter()
                                .find(|f| &f.name == field)
                                .map(|f| f.target.clone())
                                // An anonymous inline fragment stays on its parent's type.
                                .or_else(|| Some(parent.clone())),
                            (_, parent) => parent.clone(),
                        },
                    };
                    stack.push(scope);
                    last_name = None;
                    previous_was_on = false;
                }
                '}' => {
                    stack.pop();
                    last_name = None;
                    condition = None;
                    previous_was_on = false;
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let start = i;
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let name: String = chars[start..i].iter().collect();
                    if skip_name {
                        skip_name = false;
                    } else if previous_was_on {
                        condition = Some(name);
                        previous_was_on = false;
                    } else if name == "on" {
                        previous_was_on = true;
                    } else {
                        if stack.is_empty() && matches!(name.as_str(), "query" | "mutation" | "subscription") {
                            operation = Some(match name.as_str() {
                                "mutation" => "mutation",
                                "subscription" => "subscription",
                                _ => "query",
                            });
                        }
                        last_name = Some(name);
                    }
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if parens > 0 { None } else { stack.pop().flatten() }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn named(name: &str) -> Value {
        json!({ "kind": "OBJECT", "name": name, "ofType": null })
    }

    fn field(name: &str, ty: Value) -> Value {
        json!({ "name": name, "description": format!("the {name}"), "args": [], "type": ty })
    }

    fn list_of(inner: Value) -> Value {
        json!({ "kind": "LIST", "name": null, "ofType": { "kind": "NON_NULL", "name": null, "ofType": inner } })
    }

    fn sample() -> Schema {
        let body = json!({ "data": { "__schema": {
            "queryType": { "name": "Query" }, "mutationType": { "name": "Mutation" }, "subscriptionType": null,
            "types": [
                { "kind": "OBJECT", "name": "Query", "description": "", "fields": [
                    field("users", list_of(named("User"))),
                    field("viewer", named("User")),
                    { "name": "user", "description": "", "type": named("User"),
                      "args": [{ "name": "id", "type": { "kind": "NON_NULL", "name": null, "ofType": { "kind": "SCALAR", "name": "ID", "ofType": null } } }] },
                ] },
                { "kind": "OBJECT", "name": "Mutation", "fields": [ field("rename", named("User")) ] },
                { "kind": "OBJECT", "name": "User", "fields": [
                    field("id", json!({ "kind": "SCALAR", "name": "ID", "ofType": null })),
                    field("name", json!({ "kind": "SCALAR", "name": "String", "ofType": null })),
                    field("friends", list_of(named("User"))),
                    field("pet", named("Pet")),
                ] },
                { "kind": "OBJECT", "name": "Pet", "fields": [ field("nickname", json!({ "kind": "SCALAR", "name": "String", "ofType": null })) ] },
                { "kind": "OBJECT", "name": "__Schema", "fields": [] },
            ],
        } } });
        Schema::parse(&body.to_string()).unwrap()
    }

    fn labels(schema: &Schema, document: &str, marker: &str) -> Vec<String> {
        let offset = document.find(marker).unwrap();
        let document = document.replacen(marker, "", 1);
        schema.completions(&document, offset).into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn introspection_becomes_types_and_roots() {
        let schema = sample();
        assert_eq!(schema.query.as_deref(), Some("Query"));
        assert_eq!(schema.subscription, None);
        assert!(!schema.types.contains_key("__Schema"));
        let users = &schema.types["Query"].fields[0];
        assert_eq!((users.ty.as_str(), users.target.as_str()), ("[User!]", "User"));
        let user = &schema.types["Query"].fields[2];
        assert_eq!(user.args, vec![Argument { name: "id".into(), ty: "ID!".into() }]);
        assert_eq!(schema.roots()[1].1.map(|t| t.name.as_str()), Some("Mutation"));
        assert!(schema.roots()[2].1.is_none());
    }

    #[test]
    fn a_response_without_a_schema_says_why() {
        assert_eq!(
            Schema::parse(r#"{"errors":[{"message":"introspection disabled"}]}"#),
            Err("introspection disabled".into())
        );
        assert!(Schema::parse("<html>").is_err());
        assert!(Schema::parse(r#"{"data":null}"#).is_err());
    }

    #[test]
    fn the_top_level_set_offers_query_fields_filtered_by_the_typed_word() {
        let schema = sample();
        assert_eq!(labels(&schema, "{ us| }", "|"), ["users", "user"]);
        assert_eq!(labels(&schema, "query Q($a: Int) { | }", "|"), ["users", "viewer", "user", "__typename"]);
        assert_eq!(labels(&schema, "mutation { | }", "|"), ["rename", "__typename"]);
    }

    #[test]
    fn a_nested_set_offers_the_fields_of_the_type_it_selects_on() {
        let schema = sample();
        assert_eq!(labels(&schema, "{ users { na| } }", "|"), ["name"]);
        assert_eq!(labels(&schema, "{ user(id: \"1\") { friends { pet { | } } } }", "|"), ["nickname", "__typename"]);
        assert_eq!(labels(&schema, "{ me: viewer { pe| } }", "|"), ["pet"]);
        assert_eq!(labels(&schema, "{ viewer @include(if: true) { nam| } }", "|"), ["name"]);
    }

    #[test]
    fn fragments_follow_their_type_condition() {
        let schema = sample();
        assert_eq!(labels(&schema, "{ viewer { ... on Pet { nick| } } }", "|"), ["nickname"]);
        assert_eq!(labels(&schema, "fragment P on Pet { | }", "|"), ["nickname", "__typename"]);
        assert_eq!(labels(&schema, "{ viewer { ... { na| } } }", "|"), ["name"]);
    }

    #[test]
    fn nothing_is_offered_outside_a_selection_set_or_in_arguments_comments_and_strings() {
        let schema = sample();
        assert!(labels(&schema, "query |", "|").is_empty());
        assert!(labels(&schema, "{ user(id: |) { id } }", "|").is_empty());
        assert!(labels(&schema, "{ user(id: \"a|\") { id } }", "|").is_empty());
        assert_eq!(labels(&schema, "{ # users {\n vie| }", "|"), ["viewer"]);
        assert!(labels(&schema, "{ viewer { id } } |", "|").is_empty());
    }
}
