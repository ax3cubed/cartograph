<?php

use GuzzleHttp\Client;

$client = new Client(['base_uri' => 'http://orders:8080']);
$client->get('/orders/1');
$client->request('POST', '/orders');
$other->get('http://payments:8080/charge');
