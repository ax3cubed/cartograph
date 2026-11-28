; protoc-gen-connect-es v1 (XS-18): the service object spells the service and each rpc, and the
; client calls the rpc by the method's key. The fact is the rpc's own entry, one per rpc.
;   export const Orders = { typeName: "acme.orders.v1.Orders",
;     methods: { get: { name: "Get", I: GetRequest, O: Order, kind: MethodKind.Unary } } } as const;
(object
  (pair key: (property_identifier) @_t value: (string) @symbol.def.service)
  (pair
    key: (property_identifier) @_m
    value: (object
      (pair
        value: (object
          (pair key: (property_identifier) @_n value: (string) @symbol.def.rpc))) @symbol.def))
  (#eq? @_t "typeName")
  (#eq? @_m "methods")
  (#eq? @_n "name")
  (#set! method "RPC"))
