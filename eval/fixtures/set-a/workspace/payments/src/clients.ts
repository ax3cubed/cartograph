import axios from "axios";
import got from "got";

export const order = (oid: string) => axios.get(`http://orders:3000/orders/${oid}`);
export const payer = (uid: string) => fetch("http://users:3000/users/" + uid);
export const hook = () =>
  fetch("http://orders:3000/hooks/payment-updated", { method: "POST" });
// An external host: no service of ours answers to it, so it must not link.
export const card = (body: unknown) => got.post("https://api.stripe.com/v1/charges", { json: body });
// A name no compose file knows.
export const invoice = () => fetch("http://billing:3000/invoices");
