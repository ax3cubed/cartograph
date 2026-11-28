import requests
import httpx

api = httpx.Client(base_url="http://orders:8080")


def load(order_id):
    requests.get(f"http://orders:8080/orders/{order_id}")
    requests.post("http://orders:8080/orders", json={})
    return api.get("/orders/1")
