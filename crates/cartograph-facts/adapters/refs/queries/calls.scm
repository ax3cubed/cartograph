; The names a file calls, once each (XS-11): a call to a generated client's function is a call to
; its spec operation, which the index works out when it knows the generated names.
(call_expression function: (identifier) @symbol.ref @symbol.ref.name)
(call_expression function: (member_expression property: (property_identifier) @symbol.ref @symbol.ref.name))
