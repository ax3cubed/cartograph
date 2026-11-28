package main

import "net/http"

func load(id string) {
	http.Get("http://orders:8080/orders/" + id)
	http.Post("http://orders:8080/orders", "application/json", nil)
	req, _ := http.NewRequest("DELETE", "http://orders:8080/orders/1", nil)
	_ = req
}
