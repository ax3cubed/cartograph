; r.GET("/orders/:id", handler)
(
  (call_expression
    function: (selector_expression
      operand: (identifier) @route.router
      field: (field_identifier) @route.method)
    arguments: (argument_list . (interpreted_string_literal) @route.path)) @route
  (#any-of? @route.method "GET" "POST" "PUT" "PATCH" "DELETE" "HEAD" "OPTIONS" "Any")
)

; v1 := r.Group("/v1"): the prefix of the routes on `v1`, after the prefix of `r`.
(
  (short_var_declaration
    left: (expression_list . (identifier) @route.router)
    right: (expression_list . (call_expression
      function: (selector_expression
        operand: (identifier) @route.parent
        field: (field_identifier) @_g)
      arguments: (argument_list . (interpreted_string_literal) @route.path)))) @route
  (#eq? @_g "Group")
  (#set! method "PREFIX")
)
