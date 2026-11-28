; oapi-codegen: `NewGetOrderRequest` builds the path with `fmt.Sprintf("/orders/%s", ...)` and the
; method in `http.NewRequest("GET", ...)`. The method and path name the spec operation.
(function_declaration
  name: (identifier) @symbol.def.symbol
  body: (block (statement_list
    (short_var_declaration
      left: (expression_list (identifier) @_v)
      right: (expression_list
        (call_expression
          arguments: (argument_list . (interpreted_string_literal) @symbol.def.path))))
    (short_var_declaration
      right: (expression_list
        (call_expression
          function: (selector_expression field: (field_identifier) @_n)
          arguments: (argument_list . (interpreted_string_literal) @symbol.def.method))))))
  (#eq? @_v "operationPath")
  (#eq? @_n "NewRequest")) @symbol.def
