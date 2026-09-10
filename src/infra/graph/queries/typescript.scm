;; Symbols
(function_declaration
  name: (identifier) @symbol.name) @symbol.kind

(method_definition
  name: (property_identifier) @symbol.name) @symbol.kind

(class_declaration
  name: (type_identifier) @symbol.name) @symbol.kind

(interface_declaration
  name: (type_identifier) @symbol.name) @symbol.kind

(type_alias_declaration
  name: (type_identifier) @symbol.name) @symbol.kind

(enum_declaration
  name: (identifier) @symbol.name) @symbol.kind

;; `const x = () => {}`, `const x = function () {}`, `const x = { ... }`.
;; React components, Fastify plugins, hooks, and schema objects use this form.
(variable_declarator
  name: (identifier) @symbol.name) @symbol.kind

;; Calls
(call_expression
  function: [
    (identifier) @call.target
    (member_expression
      object: [
        (identifier) @call.receiver
        (this) @call.receiver
        (member_expression) @call.receiver
      ]
      property: (property_identifier) @call.target)
  ])

;; Imports
(import_statement
  source: (string) @import.source)

(import_specifier
  name: (identifier) @import.name)

(import_clause
  (identifier) @import.name)
