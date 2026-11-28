; App Router: export async function GET(), export const POST = ...
(
  (export_statement declaration: (function_declaration name: (identifier) @route.method)) @route
  (#any-of? @route.method "GET" "POST" "PUT" "PATCH" "DELETE" "HEAD" "OPTIONS")
)
(
  (export_statement declaration: (lexical_declaration
    (variable_declarator name: (identifier) @route.method))) @route
  (#any-of? @route.method "GET" "POST" "PUT" "PATCH" "DELETE" "HEAD" "OPTIONS")
)

; Pages Router API route: export default function handler(req, res)
(
  (export_statement "default") @route
  (#set! method "ANY")
)
