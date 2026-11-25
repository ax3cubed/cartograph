import axios from "axios";

// No host: the browser reaches the gateway through a reverse proxy.
export const order = (id: string) => fetch("/api/orders/" + id);
export const user = () => fetch("/api/users/42");
export const create = (body: unknown) => axios.post("/api/orders", body);
// Three services have this route and the call names none of them.
export const ping = () => fetch("/health");
