#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, SymbolKind};

#[test]
fn test_rust_symbols_and_local_calls() {
    let code = r#"
/// A helper function.
fn helper() {}

pub fn main() {
helper();
}
"#;
    let res = extract_file("my-repo", "src/main.rs", code, SupportedLanguage::Rust)
        .expect("extraction failed");

    assert_eq!(res.symbols.len(), 2);
    let helper = &res.symbols[0];
    assert_eq!(helper.name, "helper");
    assert_eq!(helper.kind, SymbolKind::Fn);
    assert!(!helper.is_exported);
    assert_eq!(helper.docstring.as_deref(), Some("/// A helper function."));

    let main_sym = &res.symbols[1];
    assert_eq!(main_sym.name, "main");
    assert_eq!(main_sym.kind, SymbolKind::Fn);
    assert!(main_sym.is_exported);

    // Check call edge
    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("helper"));
    assert_eq!(call_edge.provenance, Provenance::Extracted);
    assert!((call_edge.confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_typescript_imports_and_calls() {
    let code = r#"
import { run } from './runner';

export function execute() {
run();
}
"#;
    let res = extract_file(
        "ts-repo",
        "src/index.ts",
        code,
        SupportedLanguage::TypeScript,
    )
    .expect("extraction failed");

    assert_eq!(res.symbols.len(), 1);
    let exec_sym = &res.symbols[0];
    assert_eq!(exec_sym.name, "execute");
    assert_eq!(exec_sym.kind, SymbolKind::Fn);
    assert!(exec_sym.is_exported);

    // Check import edge
    let import_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Imports)
        .expect("import edge");
    assert_eq!(import_edge.to_name.as_deref(), Some("./runner"));
    assert_eq!(import_edge.provenance, Provenance::Extracted);
    assert!((import_edge.confidence - 0.95).abs() < f64::EPSILON);

    // Check calls edge to imported function
    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("run"));
    assert_eq!(call_edge.provenance, Provenance::Extracted);
    assert!((call_edge.confidence - 0.95).abs() < f64::EPSILON);
}

#[test]
fn test_method_call_on_receiver() {
    let code = r#"
fn process(runner: &Runner) {
runner.execute();
}
"#;
    let res = extract_file("my-repo", "src/lib.rs", code, SupportedLanguage::Rust)
        .expect("extraction failed");

    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("runner.execute"));
    assert_eq!(call_edge.provenance, Provenance::Inferred);
    assert!((call_edge.confidence - 0.85).abs() < f64::EPSILON);
}

#[test]
fn test_span_coordinates() {
    let code = "fn foo() {}\n";
    let res =
        extract_file("repo", "foo.rs", code, SupportedLanguage::Rust).expect("extraction failed");

    assert_eq!(res.symbols.len(), 1);
    let sym = &res.symbols[0];
    assert_eq!(sym.span.start_line, 1);
    assert_eq!(sym.span.start_col, 1);
    assert_eq!(sym.span.end_line, 1);
    assert_eq!(sym.span.end_col, 12);
}

#[test]
fn test_rust_cyclomatic_complexity() {
    let code = r#"
fn simple_fn() -> i32 {
    42
}

struct MyStruct {
    x: i32,
}

fn complex_fn(x: i32) -> i32 {
    if x > 10 && x < 20 {
        return 1;
    }
    match x {
        1 => 10,
        2 => 20,
        _ => 0,
    }
    while x > 0 {
        println!("{}", x);
    }
    for i in 0..5 {
        println!("{}", i);
    }
    x
}
"#;
    let res = extract_file("repo", "src/lib.rs", code, SupportedLanguage::Rust).expect("extract");

    let simple = res
        .symbols
        .iter()
        .find(|s| s.name == "simple_fn")
        .expect("simple_fn");
    assert_eq!(simple.complexity, Some(1));

    let struct_sym = res
        .symbols
        .iter()
        .find(|s| s.name == "MyStruct")
        .expect("MyStruct");
    assert_eq!(struct_sym.complexity, None);

    let complex = res
        .symbols
        .iter()
        .find(|s| s.name == "complex_fn")
        .expect("complex_fn");
    // Base: 1 + if(1) + &&(1) + match arms(3) + while(1) + for(1) = 8
    assert_eq!(complex.complexity, Some(8));
}

#[test]
fn test_typescript_cyclomatic_complexity() {
    let code = r#"
function simple(): number {
    return 1;
}

function branching(x: number, y: boolean): number {
    if (x > 0 || y) {
        for (let i = 0; i < 5; i++) {
            x += i;
        }
    }
    switch (x) {
        case 1:
            return 10;
        case 2:
            return 20;
        default:
            return x > 5 ? 100 : 0;
    }
}
"#;
    let res =
        extract_file("repo", "src/mod.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let simple = res
        .symbols
        .iter()
        .find(|s| s.name == "simple")
        .expect("simple");
    assert_eq!(simple.complexity, Some(1));

    let branching = res
        .symbols
        .iter()
        .find(|s| s.name == "branching")
        .expect("branching");
    // Base: 1 + if(1) + ||(1) + for(1) + case(2) + ternary(1) = 7
    assert_eq!(branching.complexity, Some(7));
}

#[test]
fn test_rust_hierarchy_implements() {
    let code = r#"
pub struct Dog;
pub trait Animal {}

impl Animal for Dog {}
"#;
    let res = extract_file("repo", "src/dog.rs", code, SupportedLanguage::Rust).expect("extract");

    let impl_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Implements)
        .expect("implements edge");
    assert_eq!(impl_edge.to_name.as_deref(), Some("Animal"));
    assert_eq!(impl_edge.provenance, Provenance::Extracted);
}

#[test]
fn test_typescript_hierarchy_inherits_and_implements() {
    let code = r#"
export class Dog extends Animal implements Runnable, Serializable {
    bark() {}
}

export interface Cat extends Animal, Pet {
    meow(): void;
}
"#;
    let res =
        extract_file("repo", "src/pets.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let inherits_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Inherits)
        .collect();
    assert!(
        inherits_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Animal")),
        "inherits edge to Animal"
    );

    let implements_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Implements)
        .collect();
    assert!(
        implements_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Runnable")),
        "implements edge to Runnable"
    );
    assert!(
        implements_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Serializable")),
        "implements edge to Serializable"
    );
}

#[test]
fn a_tsx_component_file_yields_its_symbols() {
    // A `Query` is bound to the grammar it compiled against, so a
    // TypeScript-compiled query over a TSX tree matches nothing at all. That
    // silence is indistinguishable from an empty file to every graph query.
    let code = r#"
import React from 'react';

export interface CardProps {
  title: string;
}

export const Card = ({ title }: CardProps) => {
  return <div className="card">{title}</div>;
};

export function useCard() {
  return null;
}
"#;
    let res = extract_file("repo", "src/Card.tsx", code, SupportedLanguage::Tsx).expect("extract");

    let names: Vec<&str> = res.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"CardProps"), "interface, got {names:?}");
    assert!(names.contains(&"Card"), "arrow component, got {names:?}");
    assert!(names.contains(&"useCard"), "hook, got {names:?}");
}

#[test]
fn a_module_level_const_arrow_function_is_a_function_symbol() {
    let code = r#"
export const authRoutes = async (fastify) => {
  fastify.post('/auth/login', handler);
};

export const SESSION_COOKIE = 'sid';
"#;
    let res = extract_file("repo", "src/routes.ts", code, SupportedLanguage::TypeScript)
        .expect("extract");

    let routes = res
        .symbols
        .iter()
        .find(|s| s.name == "authRoutes")
        .expect("arrow-function const is a symbol");
    assert_eq!(routes.kind, SymbolKind::Fn, "it binds a function");
    assert!(routes.is_exported, "the enclosing declaration is exported");

    let cookie = res
        .symbols
        .iter()
        .find(|s| s.name == "SESSION_COOKIE")
        .expect("value const is a symbol");
    assert_eq!(cookie.kind, SymbolKind::Const, "it binds a value");
}

#[test]
fn a_local_binding_inside_a_function_is_not_a_symbol() {
    // Every `const` in every function body would bury the real definitions.
    let code = r#"
export function handler() {
  const temporary = 1;
  const callback = () => temporary;
  return callback();
}
"#;
    let res = extract_file(
        "repo",
        "src/handler.ts",
        code,
        SupportedLanguage::TypeScript,
    )
    .expect("extract");

    let names: Vec<&str> = res.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"handler"),
        "the function itself, got {names:?}"
    );
    assert!(!names.contains(&"temporary"), "local value, got {names:?}");
    assert!(!names.contains(&"callback"), "local closure, got {names:?}");
}
#[test]
fn test_tsx_component_elements_are_calls_and_dom_elements_are_not() {
    let code = r#"
import { UserList } from './UserList';

export function AdminPage() {
    return <div><UserList users={[]} /><Header>title</Header></div>;
}
"#;
    let res =
        extract_file("web", "src/AdminPage.tsx", code, SupportedLanguage::Tsx).expect("extract");

    let mut called: Vec<&str> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .filter_map(|e| e.to_name.as_deref())
        .collect();
    called.sort_unstable();
    assert_eq!(called, vec!["Header", "UserList"]);
}

#[test]
fn test_http_client_calls_become_edges_to_method_and_path() {
    let code = r#"
export function listProjects() {
    return apiRequest<Project[]>('/projects');
}
export function createProject(body: NewProject) {
    return apiRequest<Project>('/projects', { method: 'POST', body: JSON.stringify(body) });
}
export function getProject(id: string) {
    return apiRequest(`/projects/${id}?expand=owner`);
}
export function health() {
    return fetch('/health');
}
export function removeUser(id: string) {
    return axios.delete(`/admin/users/${id}`);
}
export function notHttp() {
    return formatLabel('/not/a/request');
}
"#;
    let res = extract_file(
        "web",
        "src/api/projects.ts",
        code,
        SupportedLanguage::TypeScript,
    )
    .expect("extract");

    let mut requests: Vec<&str> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::CrossCallsHttp)
        .filter_map(|e| e.to_name.as_deref())
        .collect();
    requests.sort_unstable();
    assert_eq!(
        requests,
        vec![
            "DELETE /admin/users/:param",
            "GET /health",
            "GET /projects",
            "GET /projects/:param",
            "POST /projects",
        ]
    );
    assert!(
        !res.symbols
            .iter()
            .any(|s| matches!(&s.kind, SymbolKind::Other(k) if k == "route")),
        "a client call is not a route definition"
    );
}

#[test]
fn test_typescript_imported_names_are_references_at_their_import_line() {
    let code = r#"
import { Session, evaluateAccess } from '../auth/access';
import React from 'react';

export function guard(s: Session) {
    return evaluateAccess(s);
}
"#;
    let res =
        extract_file("web", "src/guard.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let mut imported: Vec<(&str, usize)> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::References && e.line <= 3)
        .filter_map(|e| e.to_name.as_deref().map(|name| (name, e.line)))
        .collect();
    imported.sort_unstable();
    assert_eq!(
        imported,
        vec![("React", 3), ("Session", 2), ("evaluateAccess", 2)]
    );
}

#[test]
fn test_http_route_symbols_extraction() {
    let code = r#"
import { FastifyInstance } from 'fastify';

export async function routes(fastify: FastifyInstance) {
  fastify.get('/health', handler);
  fastify.post('/auth/login', async (req, reply) => {
    return { status: 'ok' };
  });
  fastify.put('/users/:id', updateUser);
  fastify.patch('/users/:id', patchUser);
  fastify.delete('/users/:id', deleteUser);
  fastify.options('/cors', corsHandler);
  fastify.head('/ping', pingHandler);

  // Negative cases:
  fastify.customMethod('/not-http', handler);
  fastify.get(dynamicPath, handler);
  fastify.get();
  doSomething('/not-a-route');
}
"#;
    let res = extract_file("repo", "src/routes.ts", code, SupportedLanguage::TypeScript)
        .expect("extract");

    let route_symbols: Vec<&Symbol> = res
        .symbols
        .iter()
        .filter(|s| matches!(&s.kind, SymbolKind::Other(k) if k == "route"))
        .collect();

    let route_names: Vec<&str> = route_symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(
        route_names.contains(&"GET /health"),
        "expected GET /health, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"POST /auth/login"),
        "expected POST /auth/login, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"PUT /users/:id"),
        "expected PUT /users/:id, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"PATCH /users/:id"),
        "expected PATCH /users/:id, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"DELETE /users/:id"),
        "expected DELETE /users/:id, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"OPTIONS /cors"),
        "expected OPTIONS /cors, got {route_names:?}"
    );
    assert!(
        route_names.contains(&"HEAD /ping"),
        "expected HEAD /ping, got {route_names:?}"
    );

    // Negatives
    assert!(
        !route_names.iter().any(|n| n.contains("/not-http")),
        "should not extract non-http methods: {route_names:?}"
    );
    assert!(
        !route_names.iter().any(|n| n.contains("dynamicPath")),
        "should not extract dynamic paths: {route_names:?}"
    );
    assert!(
        !route_names.iter().any(|n| n.contains("/not-a-route")),
        "should not extract plain calls: {route_names:?}"
    );

    // Symbol properties
    let health = route_symbols
        .iter()
        .find(|s| s.name == "GET /health")
        .unwrap();
    assert_eq!(health.kind, SymbolKind::Other("route".to_owned()));
    assert!(!health.is_exported);
    assert!(health.span.start_line > 0);

    // Ensure no duplicates
    assert_eq!(
        route_symbols.len(),
        7,
        "expected exactly 7 routes, got {route_names:?}"
    );
}

#[test]
fn test_typescript_type_references_extraction() {
    let code = r#"
import { Session, User, Config } from './types';

export interface ServiceConfig {
    session: Session;
}

export function handleSession(s: Session): Promise<User> {
    const currentSession: Session = s;
    const users: Array<User> = [];
    const cfg: Config = { timeout: 1000 };
    return Promise.resolve(users[0]);
}

export class AuthHandler {
    private session: Session;
    constructor(s: Session) {
        this.session = s;
    }
    getSession(): Session {
        return this.session;
    }
}
"#;
    let res =
        extract_file("repo", "src/auth.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let ref_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::References)
        .collect();

    assert!(!ref_edges.is_empty(), "expected type references edges");

    // Check that Session is referenced
    let session_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("Session"))
        .collect();
    assert!(!session_refs.is_empty(), "expected references to Session");
    for edge in &session_refs {
        assert_eq!(edge.provenance, Provenance::Extracted);
        assert!((edge.confidence - 0.95).abs() < f64::EPSILON);
        assert!(edge.line > 0);
        assert!(edge.col > 0);
    }

    // Check User references (generic in Promise<User> and Array<User>)
    let user_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("User"))
        .collect();
    assert!(!user_refs.is_empty(), "expected references to User");

    // Check Promise and Array type references
    let promise_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("Promise"))
        .collect();
    assert!(!promise_refs.is_empty(), "expected references to Promise");

    // Check Config reference
    let config_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("Config"))
        .collect();
    assert!(!config_refs.is_empty(), "expected references to Config");

    // Check that interface definition name itself ('ServiceConfig') is NOT extracted as a reference
    let self_decl_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("ServiceConfig"))
        .collect();
    assert!(
        self_decl_refs.is_empty(),
        "interface declaration name should not be a reference"
    );

    // Check that class declaration name itself ('AuthHandler') is NOT extracted as a reference
    let class_decl_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("AuthHandler"))
        .collect();
    assert!(
        class_decl_refs.is_empty(),
        "class declaration name should not be a reference"
    );

    // Verify no exact duplicate edges (same line, col, to_name)
    let mut seen = std::collections::HashSet::new();
    for edge in &ref_edges {
        let key = (edge.line, edge.col, edge.to_name.clone());
        assert!(seen.insert(key), "duplicate reference edge found: {edge:?}");
    }
}

#[test]
fn test_tsx_type_references_extraction() {
    let code = r#"
import React from 'react';
import { Session, Theme } from './types';

export interface ButtonProps {
    session: Session;
    theme?: Theme;
}

export const Button: React.FC<ButtonProps> = ({ session, theme }: ButtonProps) => {
    const current: Session = session;
    return <button>{current.toString()}</button>;
};
"#;
    let res =
        extract_file("repo", "src/Button.tsx", code, SupportedLanguage::Tsx).expect("extract");

    let ref_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::References)
        .collect();

    let session_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("Session"))
        .collect();
    assert!(
        !session_refs.is_empty(),
        "expected references to Session in TSX"
    );

    let theme_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("Theme"))
        .collect();
    assert!(
        !theme_refs.is_empty(),
        "expected references to Theme in TSX"
    );

    let button_props_refs: Vec<_> = ref_edges
        .iter()
        .filter(|e| e.to_name.as_deref() == Some("ButtonProps"))
        .collect();
    assert!(
        !button_props_refs.is_empty(),
        "expected references to ButtonProps in TSX"
    );
}
