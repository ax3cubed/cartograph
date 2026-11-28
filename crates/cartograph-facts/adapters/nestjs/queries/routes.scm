; NestJS: @Controller("orders") on a class, @Get(":id") on its methods. A method's decorator is the
; route; the controller's argument is its `prefix`, which the index puts before the path.
(
  (export_statement decorator: (decorator (call_expression function: (identifier) @_c)) declaration: (class_declaration body: (class_body (decorator (call_expression
          function: (identifier) @route.method
          arguments: (arguments . (string) @route.path))) @route)))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
)

(
  (class_declaration decorator: (decorator (call_expression function: (identifier) @_c)) body: (class_body (decorator (call_expression
          function: (identifier) @route.method
          arguments: (arguments . (string) @route.path))) @route))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
)

(
  (export_statement decorator: (decorator (call_expression function: (identifier) @_c arguments: (arguments . (string) @route.prefix))) declaration: (class_declaration body: (class_body (decorator (call_expression function: (identifier) @route.method)) @route)))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
)

(
  (class_declaration decorator: (decorator (call_expression function: (identifier) @_c arguments: (arguments . (string) @route.prefix))) body: (class_body (decorator (call_expression function: (identifier) @route.method)) @route))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
)

(
  (export_statement decorator: (decorator (call_expression function: (identifier) @_c)) declaration: (class_declaration body: (class_body (decorator (call_expression
          function: (identifier) @route.method
          arguments: (arguments) @_e)) @route)))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
  (#match? @_e "^\\(\\s*\\)$")
  (#set! path "")
)

(
  (class_declaration decorator: (decorator (call_expression function: (identifier) @_c)) body: (class_body (decorator (call_expression
          function: (identifier) @route.method
          arguments: (arguments) @_e)) @route))
  (#eq? @_c "Controller")
  (#any-of? @route.method "Get" "Post" "Put" "Patch" "Delete" "Head" "Options" "All")
  (#match? @_e "^\\(\\s*\\)$")
  (#set! path "")
)
