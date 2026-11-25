import axios from "axios";

export const reserve = (sku: string) => fetch("http://inventory:8080/stock/" + sku);
export const hold = (body: unknown) => axios.post("http://inventory:8080/stock/reserve", body);
export const charge = (body: unknown) => axios.post("http://payments:3000/payments", body);
export const refund = (id: string) =>
  fetch(`http://payments:3000/payments/${id}/refund`, { method: "POST" });
export const buyer = (uid: string) => axios.get(`http://users:3000/users/${uid}`);
export const notify = () => fetch("http://notifier:8000/notify", { method: "POST" });
