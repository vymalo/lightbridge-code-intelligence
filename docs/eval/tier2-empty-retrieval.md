# Migration note — legacy JWT validator removal

This change finalizes the removal of the deprecated `LegacyJwtValidator` and its helper
`verify_legacy_jwt`. All token validation now goes through the current session path, and **no
call sites remain** in the control plane or the agent runner.

Reviewer: please confirm that no lingering references to `LegacyJwtValidator` or `verify_legacy_jwt`
remain anywhere in the codebase before this is merged.
