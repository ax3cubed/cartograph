; Express routes: app.get("/orders/:id", handler), router.post(...).
; The receiver must look like an app or a router, so `map.get("k", fallback)` isn't a route.
(
  (call_expression
    function: (member_expression
      object: (identifier) @_app
      property: (property_identifier) @route.method)
    arguments: (arguments . [(string) (template_string)] @route.path . (_))) @route
  (#any-of? @route.method "get" "post" "put" "patch" "delete" "head" "options" "all")
  (#match? @_app "^(app|router|api|server|[A-Za-z0-9_]*[Rr]outer|[A-Za-z0-9_]*[Aa]pp)$")
)

; A mount: router.use("/orders", ordersRouter). The prefix is a path, the mount the expression
; that handles it, so a later step can join the prefix onto that router's routes.
(
  (call_expression
    function: (member_expression
      object: (identifier) @_app
      property: (property_identifier) @route.method)
    arguments: (arguments . [(string) (template_string)] @route.path . (_) @route.mount)) @route
  (#eq? @route.method "use")
  (#match? @_app "^(app|router|api|server|[A-Za-z0-9_]*[Rr]outer|[A-Za-z0-9_]*[Aa]pp)$")
)
