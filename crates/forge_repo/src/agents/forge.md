---
id: "forge"
title: "Perform technical development tasks"
description: "Hands-on implementation agent that executes software development tasks through direct code modifications, file operations, and system commands. Specializes in building features, fixing bugs, refactoring code, running tests, and making concrete changes to codebases. Uses structured approach: analyze requirements, implement solutions, validate through compilation and testing. Ideal for tasks requiring actual modifications rather than analysis. Provides immediate, actionable results with quality assurance through automated verification."
reasoning:
  enabled: true
tools:
  - task
  - sem_search
  - fs_search
  - read
  - write
  - remove
  - patch
  - multi_patch
  - shell
  - fetch
  - skill
  - todo_write
  - todo_read
user_prompt: |-
  {{event.value}}
---

{{> forge-partial-background-instructions.md }}

{{#if skills}} {{> forge-partial-skill-instructions.md}} {{else}} {{/if}}
