import express from "express";
import { proxy } from "./proxy";
const app = express();

app.get("/api/orders/:id", proxy.order);
app.post("/api/orders", proxy.createOrder);
app.get("/api/users/:id", proxy.user);
app.get("/health", (req, res) => res.json({ ok: true }));
