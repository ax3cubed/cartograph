package main

import "net/http"

func main() {
	http.HandleFunc("GET /stock/{sku}", stock)
	http.HandleFunc("POST /stock/reserve", reserve)
	http.HandleFunc("GET /health", health)
	http.ListenAndServe(":8080", nil)
}

func resync() {
	http.Get("http://orders:3000/orders/42")
}
