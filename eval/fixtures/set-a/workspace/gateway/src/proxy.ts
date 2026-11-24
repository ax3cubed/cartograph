import axios from "axios";
import got from "got";

export const proxy = {
  order: (req, res) => fetch(`http://orders:3000/orders/${req.params.id}`),
  createOrder: (req, res) => axios.post("http://orders:3000/orders", req.body),
  user: (req, res) => axios.get(`http://users:3000/users/${req.params.id}`),
  profile: (req, res) => got.get("http://users:3000/users/" + req.params.id + "/profile"),
  payment: (req, res) => fetch("http://payments:3000/payments/" + req.params.id),
};
