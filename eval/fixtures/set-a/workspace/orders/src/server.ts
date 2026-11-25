import express from "express";
const app = express();

app.get("/orders/:id", (req, res) => res.json({}));
app.post("/orders", (req, res) => res.json({}));
app.get("/orders", (req, res) => res.json([]));
app.delete("/orders/:id", (req, res) => res.json({}));
app.get("/health", (req, res) => res.json({ ok: true }));
// A pattern, not a plain path: read as /hooks/ and whatever follows.
app.post("/hooks/(.*)", (req, res) => res.json({}));
