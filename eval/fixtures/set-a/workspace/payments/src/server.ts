import express from "express";
const app = express();

app.post("/payments", (req, res) => res.json({}));
app.get("/payments/:id", (req, res) => res.json({}));
app.post("/payments/:id/refund", (req, res) => res.json({}));
app.get("/health", (req, res) => res.json({ ok: true }));
