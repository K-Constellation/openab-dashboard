# OpenAB Dashboard — API Reference

## Base URL

```
http://your-server:8080/api
```

## Endpoints

### GET /api/summary

Daily usage summary for dashboard charts.

**Query params:**
- `days` (int, default: 14) — how many days to return
- `agent` (string, optional) — filter by agent name
- `provider` (string, optional) — filter by provider

**Response:**
```json
{
  "period": { "start": "2026-07-01", "end": "2026-07-14" },
  "daily": [
    {
      "date": "2026-07-14",
      "agents": [
        {
          "agent": "masami",
          "provider": "kiro",
          "credits": 45.2,
          "tokens": 128000,
          "requests": 23
        },
        {
          "agent": "chloe",
          "provider": "kiro",
          "credits": 38.7,
          "tokens": 95000,
          "requests": 18
        },
        {
          "agent": "misaki",
          "provider": "antigravity",
          "credits": null,
          "tokens": 67000,
          "requests": 12
        }
      ]
    }
  ]
}
```

### GET /api/quota

Current quota/credit status for all accounts.

**Response:**
```json
{
  "accounts": [
    {
      "id": "kiro:user",
      "provider": "kiro",
      "plan": "pro",
      "display_name": "Kiro Pro (user)",
      "credits_used": 342.5,
      "credits_total": 1000,
      "usage_percent": 34.25,
      "reset_at": "2026-08-01T00:00:00Z",
      "days_remaining": 18,
      "last_updated": "2026-07-14T10:05:00Z"
    },
    {
      "id": "agy:user@example.com",
      "provider": "antigravity",
      "plan": "pro",
      "display_name": "Antigravity Pro",
      "credits_used": null,
      "credits_total": null,
      "usage_percent": null,
      "tokens_used": 1250000,
      "reset_at": "2026-07-14T05:00:00Z",
      "days_remaining": null,
      "last_updated": "2026-07-14T10:05:00Z"
    }
  ]
}
```

### GET /api/leaderboard

Agent usage ranking for a time period.

**Query params:**
- `period` (string, default: "month") — "today", "week", "month"
- `metric` (string, default: "credits") — "credits", "tokens", "requests"

**Response:**
```json
{
  "period": "month",
  "metric": "credits",
  "ranking": [
    {
      "rank": 1,
      "agent": "masami",
      "provider": "kiro",
      "value": 520.3,
      "requests": 245,
      "tokens": 1560000
    },
    {
      "rank": 2,
      "agent": "chloe",
      "provider": "kiro",
      "value": 410.8,
      "requests": 198,
      "tokens": 1230000
    },
    {
      "rank": 3,
      "agent": "misaki",
      "provider": "antigravity",
      "value": null,
      "requests": 89,
      "tokens": 670000
    }
  ]
}
```

### GET /api/events

Recent usage events (paginated).

**Query params:**
- `limit` (int, default: 50)
- `offset` (int, default: 0)
- `agent` (string, optional)
- `provider` (string, optional)

**Response:**
```json
{
  "total": 1234,
  "events": [
    {
      "id": 5678,
      "timestamp": "2026-07-14T10:03:22Z",
      "agent": "masami",
      "provider": "kiro",
      "model": "claude-sonnet-4.6",
      "input_tokens": 3200,
      "output_tokens": 1800,
      "credits_consumed": 0.85,
      "duration_ms": 12500,
      "session_id": "a346316d-ef22-46bf-..."
    }
  ]
}
```

### GET /api/health

Collector health status.

**Response:**
```json
{
  "status": "healthy",
  "last_collection": "2026-07-14T10:05:00Z",
  "providers": {
    "kiro": { "status": "ok", "last_success": "2026-07-14T10:05:00Z" },
    "antigravity": { "status": "ok", "last_success": "2026-07-14T10:05:00Z" }
  },
  "db_size_mb": 12.3,
  "uptime_seconds": 86400
}
```
