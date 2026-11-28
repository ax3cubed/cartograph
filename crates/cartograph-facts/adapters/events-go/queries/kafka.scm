; Event facts of the Go Kafka clients (XS-16).

; sarama: &sarama.ProducerMessage{Topic: "orders"}
(
  (composite_literal
    type: (qualified_type package: (package_identifier) @_p name: (type_identifier) @_t)
    body: (literal_value (keyed_element
      (literal_element (identifier) @_k)
      (literal_element (_) @event.publish.topic)))) @event.publish
  (#eq? @_p "sarama")
  (#eq? @_t "ProducerMessage")
  (#eq? @_k "Topic")
  (#set! broker "kafka")
)

; segmentio/kafka-go: kafka.Message{Topic: "orders"}, kafka.Writer{Topic: "orders"}
(
  (composite_literal
    type: (qualified_type package: (package_identifier) @_p name: (type_identifier) @_t)
    body: (literal_value (keyed_element
      (literal_element (identifier) @_k)
      (literal_element (_) @event.publish.topic)))) @event.publish
  (#eq? @_p "kafka")
  (#any-of? @_t "Message" "Writer" "WriterConfig")
  (#eq? @_k "Topic")
  (#set! broker "kafka")
)

; segmentio/kafka-go: kafka.ReaderConfig{Topic: "orders"}
(
  (composite_literal
    type: (qualified_type package: (package_identifier) @_p name: (type_identifier) @_t)
    body: (literal_value (keyed_element
      (literal_element (identifier) @_k)
      (literal_element (_) @event.subscribe.topic)))) @event.subscribe
  (#eq? @_p "kafka")
  (#eq? @_t "ReaderConfig")
  (#eq? @_k "Topic")
  (#set! broker "kafka")
)

; sarama: consumer.ConsumePartition("orders", 0, offset)
(
  (call_expression
    function: (selector_expression field: (field_identifier) @_m)
    arguments: (argument_list . (_) @event.subscribe.topic)) @event.subscribe
  (#eq? @_m "ConsumePartition")
  (#set! broker "kafka")
)

; sarama: group.Consume(ctx, []string{"orders"}, handler)
(
  (call_expression
    function: (selector_expression field: (field_identifier) @_m)
    arguments: (argument_list
      (composite_literal
        type: (slice_type)
        body: (literal_value (literal_element (interpreted_string_literal) @event.subscribe @event.subscribe.topic)))))
  (#eq? @_m "Consume")
  (#set! broker "kafka")
)
