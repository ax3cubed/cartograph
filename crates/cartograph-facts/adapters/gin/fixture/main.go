package main

import "github.com/gin-gonic/gin"

func main() {
	r := gin.Default()
	r.GET("/health", health)
	v1 := r.Group("/v1")
	v1.GET("/orders/:id", one)
	admin := v1.Group("/admin")
	admin.POST("/orders", create)
	r.Run()
}
