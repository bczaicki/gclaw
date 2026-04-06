# Security Policy

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

If you discover a security vulnerability in gclaw, please report it responsibly by emailing:

**gclaw-security@proton.me**

Include as much of the following as you can:

- Description of the vulnerability
- Steps to reproduce or proof of concept
- Affected versions
- Potential impact

## What counts as a security issue

Given gclaw's architecture (container-sandboxed tool execution, multi-channel messaging, local LLM gateway), the following are examples of security-relevant issues:

- Container sandbox escape (breaking out of the Docker/Podman sandbox)
- Credential leakage (API keys, tokens exposed in logs, responses, or errors)
- Unauthorized access to the host filesystem from within the sandbox
- Injection attacks through channel messages that bypass the agent loop
- Denial of service against the agent gateway
- Memory safety issues in unsafe code (if any)

## Response timeline

- **Acknowledgment**: within 48 hours of report
- **Initial assessment**: within 7 days
- **Fix or mitigation**: best effort, typically within 30 days depending on severity

We will coordinate disclosure with you. We ask that you give us reasonable time to address the issue before public disclosure.

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Credit

We appreciate responsible disclosure and will credit reporters in the release notes (unless you prefer to remain anonymous).
