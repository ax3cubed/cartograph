; requests.get(url), httpx.post(url)
(
  (call
    function: (attribute
      object: (identifier) @_c
      attribute: (identifier) @http.call.method)
    arguments: (argument_list . (_) @http.call.url)) @http.call
  (#any-of? @_c "requests" "httpx")
  (#any-of? @http.call.method "get" "post" "put" "patch" "delete" "head" "options")
)

; api = httpx.Client(base_url=BASE): the base the calls through `api` share.
(
  (assignment
    left: (identifier) @client.base.name
    right: (call
      function: (attribute
        object: (identifier) @_c
        attribute: (identifier) @_k)
      arguments: (argument_list
        (keyword_argument name: (identifier) @_a value: (_) @client.base.url)))) @client.base
  (#eq? @_c "httpx")
  (#any-of? @_k "Client" "AsyncClient")
  (#eq? @_a "base_url")
)

; api.get("/orders"): kept by the index only when a client.base of that name is in the file.
(
  (call
    function: (attribute
      object: (identifier) @http.call.client
      attribute: (identifier) @http.call.method)
    arguments: (argument_list . (_) @http.call.url)) @http.call
  (#not-eq? @http.call.client "requests")
  (#not-eq? @http.call.client "httpx")
  (#any-of? @http.call.method "get" "post" "put" "patch" "delete" "head" "options")
  (#match? @http.call.url "^f?[\"']/")
)
