;; Rendering `<UserList />` runs the component, so a JSX element counts as a
;; call. Lowercase tags are DOM elements, not components.
(jsx_opening_element
  name: (identifier) @call.target
  (#match? @call.target "^[A-Z]"))

(jsx_self_closing_element
  name: (identifier) @call.target
  (#match? @call.target "^[A-Z]"))
