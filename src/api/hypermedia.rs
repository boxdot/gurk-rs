use serde::Serialize;
use std::collections::HashMap;

/// Wrapper for hypermedia-style JSON responses
#[derive(Debug, Serialize)]
pub struct HypermediaResource<T: Serialize> {
    #[serde(rename = "@type")]
    pub resource_type: &'static str,
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(flatten)]
    pub data: T,
    #[serde(rename = "@links")]
    pub links: Links,
    #[serde(rename = "@actions", skip_serializing_if = "HashMap::is_empty")]
    pub actions: HashMap<&'static str, Action>,
}

impl<T: Serialize> HypermediaResource<T> {
    pub fn new(resource_type: &'static str, id: impl Into<String>, data: T) -> Self {
        let id = id.into();
        let mut links = Links::new();
        links.insert("self", &id);
        Self {
            resource_type,
            id,
            data,
            links,
            actions: HashMap::new(),
        }
    }

    pub fn with_link(mut self, rel: &'static str, href: impl Into<String>) -> Self {
        self.links.insert(rel, href);
        self
    }

    pub fn with_action(mut self, name: &'static str, action: Action) -> Self {
        self.actions.insert(name, action);
        self
    }
}

/// Collection of links
#[derive(Clone, Debug, Default, Serialize)]
pub struct Links(HashMap<&'static str, String>);

impl Links {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert(&mut self, rel: &'static str, href: impl Into<String>) {
        self.0.insert(rel, href.into());
    }
}

/// An action that can be performed on a resource
#[derive(Clone, Debug, Serialize)]
pub struct Action {
    pub method: &'static str,
    pub href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

impl Action {
    pub fn post(href: impl Into<String>) -> Self {
        Self {
            method: "POST",
            href: href.into(),
            schema: None,
        }
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }
}

/// Collection wrapper with pagination
#[derive(Debug, Serialize)]
pub struct HypermediaCollection<T: Serialize> {
    #[serde(rename = "@type")]
    pub collection_type: &'static str,
    #[serde(rename = "@links")]
    pub links: Links,
    pub items: Vec<T>,
    pub count: usize,
    #[serde(rename = "@pagination", skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}

impl<T: Serialize> HypermediaCollection<T> {
    pub fn new(collection_type: &'static str, self_href: impl Into<String>, items: Vec<T>) -> Self {
        let count = items.len();
        let mut links = Links::new();
        links.insert("self", self_href);
        Self {
            collection_type,
            links,
            items,
            count,
            pagination: None,
        }
    }

    pub fn with_link(mut self, rel: &'static str, href: impl Into<String>) -> Self {
        self.links.insert(rel, href);
        self
    }

    pub fn with_pagination(mut self, next: Option<String>, prev: Option<String>) -> Self {
        if next.is_some() || prev.is_some() {
            self.pagination = Some(Pagination { next, prev });
        }
        self
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Pagination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
}
