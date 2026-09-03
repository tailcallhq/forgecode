# Incident Response Playbook - ForgeCode

This document outlines the procedures for responding to incidents affecting the ForgeCode platform, its toolchain, or its production services.

## 1. Severity Levels

| Level | Name         | Description                                                                 | Example Impact                                      |
|-------|--------------|-----------------------------------------------------------------------------|-----------------------------------------------------|
| **P0** | Critical     | Total system failure, data loss, or critical security vulnerability.        | Core CLI unusable, production environment down.     |
| **P1** | High         | Major feature failure or significant performance degradation.               | Build pipeline stalled, authentication service down.|
| **P2** | Medium       | Minor feature failure or degraded performance affecting a subset of users.  | Specific plugin failing, non-critical API errors.    |
| **P3** | Low          | Minor issues, cosmetic bugs, or non-urgent feature requests.               | UI glitch, documentation error, minor typo.         |

## 2. Response Times

| Severity | Acknowledgment | First Response | Resolution Target |
|----------|----------------|----------------|-------------------|
| **P0**   | 15 minutes     | 30 minutes     | 4 hours           |
| **P1**   | 1 hour         | 4 hours        | 24 hours          |
| **P2**   | 4 hours        | 1 business day | 3 business days   |
| **P3**   | 1 business day | 2 business days| Next release      |

## 3. Escalation Matrix

| Level | Role                                    | Contact Method       |
|-------|-----------------------------------------|----------------------|
| 1     | On-call Engineer                        | Slack / PagerDuty    |
| 2     | Lead Engineer / Technical Lead          | Phone / Slack DM     |
| 3     | Engineering Manager / Product Owner     | Emergency Call       |
| 4     | CTO / VP of Engineering                 | Executive Escalation |

## 4. Communication Templates

### Internal Notification (Slack/Teams)
```markdown
:rotating_light: **[P{X}] Incident Detected: [Brief Title]**
*Status:* Investigating
*Impact:* [Description of user impact]
*Lead:* [Name]
*Channel:* #incident-[ticket-id]
```

### External Status Page Update
```markdown
**Investigating** - We are currently investigating an issue affecting [Service Name]. 
Some users may experience [Symptom]. We will provide an update in 30 minutes.
```

### Resolution Notification
```markdown
**Resolved** - The issue affecting [Service Name] has been resolved as of [Time]. 
Root cause: [Brief RCA]. We apologize for the inconvenience.
```

## 5. Post-Mortem Template

### Incident Summary
- **Date/Time of Incident:** YYYY-MM-DD HH:MM UTC
- **Duration:** X hours Y minutes
- **Severity:** P0 / P1 / P2 / P3
- **Authors:** [List of authors]

### Impact
- **User Impact:** [Number of users affected / Revenue impact]
- **Service Impact:** [Specific services/components affected]

### Timeline (UTC)
- **HH:MM:** [Event]
- **HH:MM:** [Event]
- ...

### Root Cause Analysis
[Detailed explanation of what went wrong and why.]

### What went well?
- [Item 1]
- [Item 2]

### What didn't go well?
- [Item 1]
- [Item 2]

### Action Items
| Action Item | Owner | Priority | Status |
|-------------|-------|----------|--------|
| [Task 1]    | Name  | High     | Open   |
| [Task 2]    | Name  | Medium   | Open   |

---

## 6. Root Cause Analysis (RCA) Template

### 1. What happened?
[High-level summary of the incident.]

### 2. Why did it happen? (The "Why" Chain)
- **Problem:** [Symptom]
- **Cause 1:** [Direct cause] → Why? [Deeper reason]
- **Cause 2:** [Systemic cause] → Why? [Process failure]

### 3. Contributing Factors
- **Technical:** [e.g., Lack of retry logic, missing alerting]
- **Process:** [e.g., Inadequate testing, missed code review]
- **People:** [e.g., Lack of training, communication gap]

### 4. Corrective Actions
- **Immediate:** [Actions taken to resolve the incident]
- **Short-term:** [Actions to prevent recurrence in 1-2 weeks]
- **Long-term:** [Systemic improvements]

### 5. Lessons Learned
[Key takeaways for the team and organization.]
