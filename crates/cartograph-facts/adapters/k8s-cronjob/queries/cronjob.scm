; kind: CronJob, metadata.name and spec.schedule of one manifest.
(
  (block_mapping
    (block_mapping_pair
      key: [(flow_node (plain_scalar (string_scalar) @_k1)) (flow_node (double_quote_scalar) @_k1) (flow_node (single_quote_scalar) @_k1)]
      value: [(flow_node (plain_scalar (string_scalar) @_v1)) (flow_node (double_quote_scalar) @_v1) (flow_node (single_quote_scalar) @_v1)]) @entrypoint
    (block_mapping_pair
      key: [(flow_node (plain_scalar (string_scalar) @_k2)) (flow_node (double_quote_scalar) @_k2) (flow_node (single_quote_scalar) @_k2)]
      value: (block_node (block_mapping (block_mapping_pair
        key: [(flow_node (plain_scalar (string_scalar) @_k3)) (flow_node (double_quote_scalar) @_k3) (flow_node (single_quote_scalar) @_k3)]
        value: [(flow_node (plain_scalar (string_scalar) @entrypoint.name)) (flow_node (double_quote_scalar) @entrypoint.name) (flow_node (single_quote_scalar) @entrypoint.name)]))))
    (block_mapping_pair
      key: [(flow_node (plain_scalar (string_scalar) @_k4)) (flow_node (double_quote_scalar) @_k4) (flow_node (single_quote_scalar) @_k4)]
      value: (block_node (block_mapping (block_mapping_pair
        key: [(flow_node (plain_scalar (string_scalar) @_k5)) (flow_node (double_quote_scalar) @_k5) (flow_node (single_quote_scalar) @_k5)]
        value: [(flow_node (plain_scalar (string_scalar) @entrypoint.schedule)) (flow_node (double_quote_scalar) @entrypoint.schedule) (flow_node (single_quote_scalar) @entrypoint.schedule)])))))
  (#eq? @_k1 "kind")
  (#eq? @_v1 "CronJob")
  (#eq? @_k2 "metadata")
  (#eq? @_k3 "name")
  (#eq? @_k4 "spec")
  (#eq? @_k5 "schedule")
  (#set! kind "job")
)
