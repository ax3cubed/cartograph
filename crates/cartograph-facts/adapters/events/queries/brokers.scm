; Event facts (XS-16): what a file publishes to and subscribes to. `broker` is the channel's kind,
; `topic` the channel as the code spells it (a string, parts, or an expression).

; kafkajs: producer.send({ topic: "orders", messages })
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.publish.topic)))) @event.publish
  (#eq? @_m "send")
  (#eq? @_k "topic")
  (#match? @_r "[Pp]roducer")
  (#set! broker "kafka")
)

; kafkajs: consumer.subscribe({ topic: "orders" })
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.subscribe.topic)))) @event.subscribe
  (#eq? @_m "subscribe")
  (#eq? @_k "topic")
  (#match? @_r "[Cc]onsumer")
  (#set! broker "kafka")
)

; kafkajs: consumer.subscribe({ topics: ["a", "b"] }), one fact per topic
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k
      value: (array (_) @event.subscribe @event.subscribe.topic)))))
  (#eq? @_m "subscribe")
  (#eq? @_k "topics")
  (#match? @_r "[Cc]onsumer")
  (#set! broker "kafka")
)

; amqplib: channel.publish("exchange", "routing.key", content): the routing key is the channel
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) . (_) @event.publish.topic)) @event.publish
  (#eq? @_m "publish")
  (#match? @_r "[Cc]hannel|^ch$")
  (#set! broker "amqp")
)

; amqplib: channel.sendToQueue("queue", content)
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) @event.publish.topic)) @event.publish
  (#eq? @_m "sendToQueue")
  (#set! broker "amqp")
)

; amqplib: channel.consume("queue", handler)
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) @event.subscribe.topic)) @event.subscribe
  (#eq? @_m "consume")
  (#match? @_r "[Cc]hannel|^ch$")
  (#set! broker "amqp")
)

; amqplib: channel.bindQueue("queue", "exchange", "order.*"): the pattern a queue hears
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) . (_) . (_) @event.subscribe.topic)) @event.subscribe
  (#eq? @_m "bindQueue")
  (#set! broker "amqp")
)

; BullMQ: new Queue("emails") produces to it, new Worker("emails", handler) consumes from it
(
  (new_expression
    constructor: (identifier) @_c
    arguments: (arguments . (_) @event.publish.topic)) @event.publish
  (#eq? @_c "Queue")
  (#set! broker "bullmq")
)
(
  (new_expression
    constructor: (identifier) @_c
    arguments: (arguments . (_) @event.subscribe.topic)) @event.subscribe
  (#eq? @_c "Worker")
  (#set! broker "bullmq")
)

; AWS SDK v3: new SendMessageCommand({ QueueUrl }), new PublishCommand({ TopicArn })
(
  (new_expression
    constructor: (identifier) @_c
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.publish.topic)))) @event.publish
  (#any-of? @_c "SendMessageCommand" "SendMessageBatchCommand")
  (#eq? @_k "QueueUrl")
  (#set! broker "sqs")
)
(
  (new_expression
    constructor: (identifier) @_c
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.publish.topic)))) @event.publish
  (#any-of? @_c "PublishCommand" "PublishBatchCommand")
  (#eq? @_k "TopicArn")
  (#set! broker "sns")
)
(
  (new_expression
    constructor: (identifier) @_c
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.subscribe.topic)))) @event.subscribe
  (#eq? @_c "ReceiveMessageCommand")
  (#eq? @_k "QueueUrl")
  (#set! broker "sqs")
)

; AWS SDK v2: sqs.sendMessage({ QueueUrl }), sns.publish({ TopicArn }), sqs.receiveMessage({ QueueUrl })
(
  (call_expression
    function: (member_expression property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.publish.topic)))) @event.publish
  (#any-of? @_m "sendMessage" "sendMessageBatch")
  (#eq? @_k "QueueUrl")
  (#set! broker "sqs")
)
(
  (call_expression
    function: (member_expression property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.publish.topic)))) @event.publish
  (#eq? @_m "publish")
  (#eq? @_k "TopicArn")
  (#set! broker "sns")
)
(
  (call_expression
    function: (member_expression property: (property_identifier) @_m)
    arguments: (arguments (object (pair key: (property_identifier) @_k value: (_) @event.subscribe.topic)))) @event.subscribe
  (#eq? @_m "receiveMessage")
  (#eq? @_k "QueueUrl")
  (#set! broker "sqs")
)

; NATS: nc.publish("orders.paid", data), nc.subscribe("orders.*")
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) @event.publish.topic)) @event.publish
  (#eq? @_m "publish")
  (#match? @_r "^(nc|js|nats|jetstream|[A-Za-z]*Nc|[A-Za-z]*[Nn]ats[A-Za-z]*)$")
  (#set! broker "nats")
)
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) @event.subscribe.topic)) @event.subscribe
  (#eq? @_m "subscribe")
  (#match? @_r "^(nc|js|nats|jetstream|[A-Za-z]*Nc|[A-Za-z]*[Nn]ats[A-Za-z]*)$")
  (#set! broker "nats")
)

; Redis pub/sub: redis.publish("orders", msg), sub.subscribe("orders"), sub.psubscribe("order*")
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments . (_) @event.publish.topic)) @event.publish
  (#eq? @_m "publish")
  (#match? @_r "[Rr]edis|^(pub|publisher)$")
  (#set! broker "redis")
)
(
  (call_expression
    function: (member_expression object: (_) @_r property: (property_identifier) @_m)
    arguments: (arguments [(string) (template_string)] @event.subscribe @event.subscribe.topic))
  (#any-of? @_m "subscribe" "psubscribe")
  (#match? @_r "[Rr]edis|^(sub|subscriber)$")
  (#set! broker "redis")
)
