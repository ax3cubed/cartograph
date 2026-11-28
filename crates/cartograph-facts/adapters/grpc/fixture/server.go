package main

import (
	"google.golang.org/grpc"

	pb "example.com/orders/pb"
)

func main() {
	s := grpc.NewServer()
	pb.RegisterOrdersServer(s, &orders{})
	pb.RegisterHealthServer(s, &health{})
}
