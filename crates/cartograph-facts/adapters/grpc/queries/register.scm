; pb.RegisterOrdersServer(server, impl): this program serves the service Orders.
(
  (call_expression
    function: (selector_expression field: (field_identifier) @entrypoint.register)) @entrypoint
  (#match? @entrypoint.register "^Register[A-Za-z0-9_]+Server$")
  (#set! kind "grpc")
)
