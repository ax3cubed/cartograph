// Demo symbols for northwind (G11). The map stops at files; symbols appear only in a file's focus view.
// Every symbol call sits on top of a file link that already exists in lib/workspace.ts.
export interface Sym {
  id: string; name: string; kind: "function" | "type"; file: string; lines: [number, number];
  sig: string; summary: string; // summary is written by a model
}
export interface SymLink { from: string; to: string; kind: "call" | "type" }

const B = "billing", G = "gateway", S = "shared", C = "catalog";
const sym = (file: string, name: string, kind: Sym["kind"], lines: [number, number], sig: string, summary: string): Sym =>
  ({ id: `${file}#${name}`, name, kind, file, lines, sig, summary });

export const SYMBOLS: Sym[] = [
  sym(`${B}/invoices/createInvoice.ts`, "createInvoice", "function", [24, 71], "export async function createInvoice(\n  order: Order, opts?: InvoiceOpts\n): Promise<Invoice>", "Prices each order line through the catalog API, then stores the invoice with the next number in sequence."),
  sym(`${B}/invoices/createInvoice.ts`, "nextNumber", "function", [74, 88], "function nextNumber(): Promise<string>", "Reserves the next invoice number for the current year."),
  sym(`${B}/invoices/pdf.ts`, "renderPdf", "function", [12, 58], "export function renderPdf(inv: Invoice): Buffer", "Lays out an invoice as a PDF for the email attachment."),
  sym(`${B}/payments/charge.ts`, "chargeInvoice", "function", [18, 49], "export async function chargeInvoice(inv: Invoice): Promise<Payment>", "Charges the saved payment method and publishes payment.succeeded or payment.failed."),
  sym(`${S}/money/currency.ts`, "Money", "type", [4, 9], "export type Money = { minor: number; currency: string }", "Amount in minor units with its currency code."),
  sym(`${S}/money/currency.ts`, "roundMoney", "function", [22, 31], "export function roundMoney(m: Money): Money", "Rounds to the currency's minor unit, half to even."),
  sym(`${S}/money/currency.ts`, "toMinor", "function", [34, 40], "export function toMinor(amount: string, cur: string): Money", "Parses a decimal string into minor units."),
  sym(`${C}/api/products.py`, "get_product", "function", [21, 44], "def get_product(sku: str) -> Product", "Returns a product with its computed price."),
  sym(`${G}/clients/billingClient.ts`, "postInvoice", "function", [14, 33], "export function postInvoice(order: Order): Promise<Invoice>", "Sends POST /invoices to billing-service and parses the reply."),
  sym(`${G}/routes/checkout.ts`, "checkout", "function", [10, 47], "export async function checkout(req: Request): Promise<Response>", "Authenticates the caller, then asks billing to raise and charge an invoice."),
  sym(`${G}/routes/orders.ts`, "createOrder", "function", [28, 52], "export async function createOrder(req: Request): Promise<Response>", "Creates an order and forwards it to billing."),
];

const f = `${B}/invoices/createInvoice.ts#`;
export const SYMBOL_LINKS: SymLink[] = [
  { from: `${G}/clients/billingClient.ts#postInvoice`, to: `${f}createInvoice`, kind: "call" },
  { from: `${G}/routes/checkout.ts#checkout`, to: `${G}/clients/billingClient.ts#postInvoice`, kind: "call" },
  { from: `${G}/routes/orders.ts#createOrder`, to: `${G}/clients/billingClient.ts#postInvoice`, kind: "call" },
  { from: `${f}createInvoice`, to: `${f}nextNumber`, kind: "call" },
  { from: `${f}createInvoice`, to: `${B}/invoices/pdf.ts#renderPdf`, kind: "call" },
  { from: `${f}createInvoice`, to: `${B}/payments/charge.ts#chargeInvoice`, kind: "call" },
  { from: `${f}createInvoice`, to: `${S}/money/currency.ts#roundMoney`, kind: "call" },
  { from: `${f}createInvoice`, to: `${S}/money/currency.ts#Money`, kind: "type" },
  { from: `${f}createInvoice`, to: `${C}/api/products.py#get_product`, kind: "call" },
  { from: `${B}/payments/charge.ts#chargeInvoice`, to: `${S}/money/currency.ts#toMinor`, kind: "call" },
];

export const symbolsIn = (file: string) => SYMBOLS.filter((s) => s.file === file);
export const symbolById = (id: string) => SYMBOLS.find((s) => s.id === id);
