<?php

use Illuminate\Support\Facades\Route;

Route::get('/orders/{id}', [OrderController::class, 'show']);
Route::post('/orders', [OrderController::class, 'store']);

Route::prefix('admin')->group(function () {
    Route::delete('/orders/{id}', [AdminController::class, 'destroy']);
});
