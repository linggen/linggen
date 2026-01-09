---
id: security-policy
name: Security Guardrails
description: Strict security checks, secret detection, and vulnerability scanning rules.
scope: Team
version: 0.9.0
author: Security Team
tags: ["security", "compliance"]
---

# Security Guardrails

- Never commit secrets (API keys, passwords) to the repository.
- Use environment variables for sensitive configuration.
- Sanitize all user inputs to prevent XSS and SQL injection.
- Use HTTPS for all external API communications.
- Regularly update dependencies to patch known vulnerabilities.
