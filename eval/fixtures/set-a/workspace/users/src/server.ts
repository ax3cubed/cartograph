import express from "express";
const app = express();

app.get("/users/:id", (req, res) => res.json({}));
app.post("/users", (req, res) => res.json({}));
app.get("/users/:id/profile", (req, res) => res.json({}));
app.get("/health", (req, res) => res.json({ ok: true }));
