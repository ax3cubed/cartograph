; The method and path of each function of a generated client (XS-11). The function's name is an
; alias of the spec operation that has that method and path.

; const getOrder = (id) => axios.get(`/orders/${id}`)
(variable_declarator
  name: (identifier) @symbol.def.symbol
  value: (arrow_function
    body: (call_expression
      function: (member_expression
        property: (property_identifier) @symbol.def.method)
      arguments: (arguments . [(string) (template_string)] @symbol.def.path)))
  (#any-of? @symbol.def.method "get" "post" "put" "patch" "delete" "head" "options")) @symbol.def

; const getOrder = (id) => { return axios.get(`/orders/${id}`) }
(variable_declarator
  name: (identifier) @symbol.def.symbol
  value: (arrow_function
    body: (statement_block
      (return_statement
        (call_expression
          function: (member_expression
            property: (property_identifier) @symbol.def.method)
          arguments: (arguments . [(string) (template_string)] @symbol.def.path)))))
  (#any-of? @symbol.def.method "get" "post" "put" "patch" "delete" "head" "options")) @symbol.def

; const getOrder = (id) => customInstance({url: `/orders/${id}`, method: 'GET'})
(variable_declarator
  name: (identifier) @symbol.def.symbol
  value: (arrow_function
    body: (call_expression
      arguments: (arguments
        (object
          (pair key: (property_identifier) @_u value: [(string) (template_string)] @symbol.def.path)
          (pair key: (property_identifier) @_m value: (string) @symbol.def.method)))))
  (#eq? @_u "url")
  (#eq? @_m "method")) @symbol.def

(variable_declarator
  name: (identifier) @symbol.def.symbol
  value: (arrow_function
    body: (statement_block
      (return_statement
        (call_expression
          arguments: (arguments
            (object
              (pair key: (property_identifier) @_u value: [(string) (template_string)] @symbol.def.path)
              (pair key: (property_identifier) @_m value: (string) @symbol.def.method)))))))
  (#eq? @_u "url")
  (#eq? @_m "method")) @symbol.def
