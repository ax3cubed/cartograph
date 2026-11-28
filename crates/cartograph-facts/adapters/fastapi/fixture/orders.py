from fastapi import APIRouter, FastAPI

app = FastAPI()
router = APIRouter(prefix="/orders")


@router.get("/{order_id}")
def one(order_id: int):
    return {}


@router.post("")
def create():
    return {}


@app.get("/health")
def health():
    return "ok"


app.include_router(router, prefix="/api")
