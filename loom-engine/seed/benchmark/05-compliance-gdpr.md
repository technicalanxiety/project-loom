# GDPR audit trail and the User Data Store

The User Data Store holds every column that GDPR classifies as
personal data — names, emails, government identifiers, addresses,
and any free-text field that could contain personal information.
The store complies with our retention schedule by tagging every row
with an erasure date computed from the last-active timestamp.

The GDPR Policy governs which fields land in the User Data Store
and which fields are excluded entirely. It specifies the legal
basis for every retained field, the maximum retention window, and
the export format we must produce in response to a subject-access
request. The policy is owned by legal and reviewed quarterly; any
schema change that touches the User Data Store has to cite the
GDPR Policy clause that justifies the new field.

The audit trail for User Data Store access is append-only. Every
read of a personal-data field, whether from a service or a
human-driven query tool, writes a row into a separate audit log
that the GDPR Policy requires us to retain for seven years. The
audit log itself does not contain the personal data — only the
identity of the reader, the field name, and the lawful basis.
