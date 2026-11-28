; http.Get(url), http.Post(url, ...)
(
  (call_expression
    function: (selector_expression
      operand: (identifier) @_p
      field: (field_identifier) @http.call.method)
    arguments: (argument_list . (_) @http.call.url)) @http.call
  (#eq? @_p "http")
  (#any-of? @http.call.method "Get" "Post" "Head")
)

; http.NewRequest("DELETE", url, body)
(
  (call_expression
    function: (selector_expression
      operand: (identifier) @_p
      field: (field_identifier) @_f)
    arguments: (argument_list
      . (interpreted_string_literal) @http.call.method
      . (_) @http.call.url)) @http.call
  (#eq? @_p "http")
  (#eq? @_f "NewRequest")
)

; http.NewRequestWithContext(ctx, "DELETE", url, body)
(
  (call_expression
    function: (selector_expression
      operand: (identifier) @_p
      field: (field_identifier) @_f)
    arguments: (argument_list
      . (_)
      . (interpreted_string_literal) @http.call.method
      . (_) @http.call.url)) @http.call
  (#eq? @_p "http")
  (#eq? @_f "NewRequestWithContext")
)
