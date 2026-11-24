; fetch(url): a GET unless the options say otherwise.
(
  (call_expression
    function: (identifier) @_f
    arguments: (arguments . (_) @http.call.url)) @http.call
  (#eq? @_f "fetch")
  (#set! method "GET")
)

; fetch(url, { method: "POST" })
(
  (call_expression
    function: (identifier) @_f
    arguments: (arguments
      . (_) @http.call.url
      . (object (pair key: (property_identifier) @_k value: (string) @http.call.method)))) @http.call
  (#eq? @_f "fetch")
  (#eq? @_k "method")
)

; axios.post(url, ...), got.get(url), ky.put(url)
(
  (call_expression
    function: (member_expression
      object: (identifier) @_c
      property: (property_identifier) @http.call.method)
    arguments: (arguments . (_) @http.call.url)) @http.call
  (#any-of? @_c "axios" "got" "ky")
  (#any-of? @http.call.method "get" "post" "put" "patch" "delete" "head" "options")
)

; axios(url), got(url)
(
  (call_expression
    function: (identifier) @_c
    arguments: (arguments . (_) @http.call.url)) @http.call
  (#any-of? @_c "axios" "got")
  (#set! method "GET")
)

; axios.create({ baseURL: BASE }): the base expression the calls through that client share.
(
  (call_expression
    function: (member_expression
      object: (identifier) @_c
      property: (property_identifier) @_p)
    arguments: (arguments
      (object (pair key: (property_identifier) @_k value: (_) @client.base.url)))) @client.base
  (#eq? @_c "axios")
  (#eq? @_p "create")
  (#eq? @_k "baseURL")
)
