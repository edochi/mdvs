---
title: "API Design Principles"
tags: [api, rest, design-patterns]
date: 2025-04-22
created: 2025-04-20T09:15:00
modified: 2025-06-01T14:30:00+02:00
author: edoardo
category: engineering
status: published
draft: false
rating: 4
source: "https://example.com/api-guidelines"
project: backend-v2
priority: high
aliases:
  - "REST API Guidelines"
  - "API Standards"
cssclass: wide-page
publish: true
lang: en
custom_field: "arbitrary string value"
word_count: 312
---

# API Design Principles

A set of conventions we follow for public-facing REST endpoints.

## Use Nouns for Resources

Endpoints should represent resources, not actions. Use HTTP methods to express the operation.

| Method | Path              | Meaning             |
|--------|-------------------|---------------------|
| GET    | `/users`          | List users          |
| POST   | `/users`          | Create a user       |
| GET    | `/users/:id`      | Get a single user   |
| PATCH  | `/users/:id`      | Update a user       |
| DELETE | `/users/:id`      | Delete a user       |

## Pagination

Always paginate list endpoints. Use cursor-based pagination for large datasets and offset-based for small, rarely-changing collections.

## Versioning

Prefer URL path versioning (`/v1/users`) over header-based versioning. It is explicit, easy to route, and simple to test with curl.

## Error Responses

Return a consistent error envelope:

```json
{
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "The 'email' field must be a valid email address.",
    "details": [
      { "field": "email", "issue": "invalid_format" }
    ]
  }
}
```
