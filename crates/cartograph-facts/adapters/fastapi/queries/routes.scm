; @router.get("/orders/{id}")
(
  (decorator (call
    function: (attribute object: (identifier) @route.router attribute: (identifier) @route.method)
    arguments: (argument_list . (string) @route.path))) @route
  (#any-of? @route.method "get" "post" "put" "patch" "delete" "head" "options")
)

; router = APIRouter(prefix="/orders"): the prefix of the routes on `router` in this file.
(
  (assignment
    left: (identifier) @route.router
    right: (call
      function: (identifier) @_c
      arguments: (argument_list
        (keyword_argument name: (identifier) @_k value: (string) @route.path)))) @route
  (#eq? @_c "APIRouter")
  (#eq? @_k "prefix")
  (#set! method "PREFIX")
)

; app.include_router(router, prefix="/api"): goes before the router's own prefix.
(
  (call
    function: (attribute attribute: (identifier) @_m)
    arguments: (argument_list
      . (identifier) @route.router
      (keyword_argument name: (identifier) @_k value: (string) @route.path))) @route
  (#eq? @_m "include_router")
  (#eq? @_k "prefix")
  (#set! method "PREFIX")
  (#set! outer "1")
)
