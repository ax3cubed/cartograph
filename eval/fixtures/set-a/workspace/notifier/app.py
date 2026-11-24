import requests
from fastapi import FastAPI

app = FastAPI()


@app.post("/notify")
def notify():
    return {}


@app.get("/notify/{id}")
def status(id: str):
    return {}


def lookup():
    return requests.get("http://users:3000/users/1")
