;; Symbols
(function_item
  name: (identifier) @symbol.name) @symbol.kind

(struct_item
  name: (type_identifier) @symbol.name) @symbol.kind

(trait_item
  name: (type_identifier) @symbol.name) @symbol.kind

(enum_item
  name: (type_identifier) @symbol.name) @symbol.kind

(impl_item
  type: (type_identifier) @symbol.name) @symbol.kind

(mod_item
  name: (identifier) @symbol.name) @symbol.kind

(const_item
  name: (identifier) @symbol.name) @symbol.kind

(static_item
  name: (identifier) @symbol.name) @symbol.kind

;; Calls
(call_expression
  function: [
    (identifier) @call.target
    (field_expression
      value: [
        (identifier) @call.receiver
        (self) @call.receiver
      ]
      field: (field_identifier) @call.target)
    (scoped_identifier
      path: [
        (identifier) @call.receiver
        (scoped_identifier) @call.receiver
      ]
      name: (identifier) @call.target)
  ])

;; Imports
(use_declaration
  argument: [
    (scoped_identifier) @import.path
    (identifier) @import.path
    (use_as_clause
      path: (scoped_identifier) @import.path)
    (use_wildcard
      (scoped_identifier) @import.path)
    (use_list) @import.path
  ])
