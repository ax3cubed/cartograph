; Environment reads (XS-09): the variable a call's base URL comes from.

; process.env.KEY
(
  (member_expression
    object: (member_expression
      object: (identifier) @_p
      property: (property_identifier) @_e)
    property: (property_identifier) @env.read.key) @env.read
  (#eq? @_p "process")
  (#eq? @_e "env")
)

; process.env["KEY"]
(
  (subscript_expression
    object: (member_expression
      object: (identifier) @_p
      property: (property_identifier) @_e)
    index: (string) @env.read.key) @env.read
  (#eq? @_p "process")
  (#eq? @_e "env")
)

; import.meta.env.KEY
(
  (member_expression
    object: (member_expression
      object: (meta_property) @_m
      property: (property_identifier) @_e)
    property: (property_identifier) @env.read.key) @env.read
  (#eq? @_e "env")
)

; const BASE = process.env.KEY: the name the rest of the file reads it by.
(
  (variable_declarator
    name: (identifier) @env.read.name
    value: (member_expression
      object: (member_expression
        object: (identifier) @_p
        property: (property_identifier) @_e)
      property: (property_identifier) @env.read.key) @env.read)
  (#eq? @_p "process")
  (#eq? @_e "env")
)

; const BASE = process.env.KEY ?? "http://localhost", or with ||
(
  (variable_declarator
    name: (identifier) @env.read.name
    value: (binary_expression
      left: (member_expression
        object: (member_expression
          object: (identifier) @_p
          property: (property_identifier) @_e)
        property: (property_identifier) @env.read.key) @env.read))
  (#eq? @_p "process")
  (#eq? @_e "env")
)
