; protoc-gen-go-grpc (XS-18). Each rpc has a constant with the name gRPC puts on the wire, shared by
; the client and the server: `Orders_Get_FullMethodName = "/acme.orders.v1.Orders/Get"`.
(const_spec
  name: (identifier) @symbol.def.symbol
  value: (expression_list . (interpreted_string_literal) @symbol.def.path)
  (#match? @symbol.def.symbol "_FullMethodName$")
  (#set! method "RPC")) @symbol.def

; The service descriptor names the service a `RegisterOrdersServer(s, impl)` call serves.
(var_spec
  name: (identifier) @_v
  value: (expression_list
    (composite_literal
      body: (literal_value
        (keyed_element
          (literal_element (identifier) @_k)
          (literal_element (interpreted_string_literal) @symbol.def.path)))))
  (#match? @_v "_ServiceDesc$")
  (#eq? @_k "ServiceName")
  (#set! method "SERVE")) @symbol.def
