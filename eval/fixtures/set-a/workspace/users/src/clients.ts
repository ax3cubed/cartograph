import axios from "axios";

export const welcome = () => fetch("http://notifier:8000/notify", { method: "POST" });
export const listOrders = () => axios.get("http://orders:3000/orders");
