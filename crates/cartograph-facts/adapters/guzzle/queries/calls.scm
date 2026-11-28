; $client = new Client(['base_uri' => BASE]);
(
  (assignment_expression
    left: (variable_name) @client.base.name
    right: (object_creation_expression
      (name) @_c
      (arguments (argument (array_creation_expression
        (array_element_initializer (string) @_k (string) @client.base.url)))))) @client.base
  (#eq? @_c "Client")
  (#match? @_k "base_uri")
)

; $client->get('/orders'): through a client; kept by the index when it has a base in the file.
(
  (member_call_expression
    object: (variable_name) @http.call.client
    name: (name) @http.call.method
    arguments: (arguments . (argument (string) @http.call.url))) @http.call
  (#any-of? @http.call.method "get" "post" "put" "patch" "delete" "head" "options")
  (#match? @http.call.url "^['\"]/")
)

; $client->request('POST', '/orders')
(
  (member_call_expression
    object: (variable_name) @http.call.client
    name: (name) @_r
    arguments: (arguments
      . (argument (string) @http.call.method)
      . (argument (string) @http.call.url))) @http.call
  (#eq? @_r "request")
  (#match? @http.call.url "^['\"]/")
)

; An absolute URL needs no client.
(
  (member_call_expression
    name: (name) @http.call.method
    arguments: (arguments . (argument (string) @http.call.url))) @http.call
  (#any-of? @http.call.method "get" "post" "put" "patch" "delete" "head" "options")
  (#match? @http.call.url "^['\"]https?://")
)
(
  (member_call_expression
    name: (name) @_r
    arguments: (arguments
      . (argument (string) @http.call.method)
      . (argument (string) @http.call.url))) @http.call
  (#eq? @_r "request")
  (#match? @http.call.url "^['\"]https?://")
)
