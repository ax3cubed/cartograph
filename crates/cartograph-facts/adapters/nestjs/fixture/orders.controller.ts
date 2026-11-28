import { Controller, Get, Post, Param } from "@nestjs/common";

@Controller("orders")
export class OrdersController {
  @Get()
  list() {}

  @Get(":id")
  one(@Param("id") id: string) {}

  @Post("bulk")
  bulk() {}
}

@Controller()
class RootController {
  @Get("health")
  health() {}
}

class NotAController {
  @Get("nope")
  nope() {}
}
