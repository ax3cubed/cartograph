; Route::get('/orders/{id}', [OrderController::class, 'show']);
(
  (scoped_call_expression
    scope: (name) @_s
    name: (name) @route.method
    arguments: (arguments . (argument (string) @route.path))) @route
  (#eq? @_s "Route")
  (#any-of? @route.method "get" "post" "put" "patch" "delete" "options" "any")
)

; Route::prefix('admin')->group(function () { Route::get(...); }): one level of group.
(
  (member_call_expression
    object: (scoped_call_expression
      scope: (name) @_s
      name: (name) @_p
      arguments: (arguments . (argument (string) @route.prefix)))
    name: (name) @_g
    arguments: (arguments (argument (anonymous_function
      body: (compound_statement (expression_statement (scoped_call_expression
        scope: (name) @_s2
        name: (name) @route.method
        arguments: (arguments . (argument (string) @_path))) @route)))))
  )
  (#eq? @_s "Route")
  (#eq? @_s2 "Route")
  (#eq? @_p "prefix")
  (#eq? @_g "group")
  (#any-of? @route.method "get" "post" "put" "patch" "delete" "options" "any")
)
