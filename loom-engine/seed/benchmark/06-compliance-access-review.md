# Production DB access controls

Every connection to the Production DB is authorized by an IAM
Policy attached to the connecting principal — there are no
shared credentials, no service-account passwords in environment
files, and no break-glass account. The IAM Policy restricts each
principal to a specific schema and a specific set of statement
classes (read-only, read-write, DDL).

Access reviews for the Production DB run on a quarterly cycle.
Every principal that holds a non-read-only IAM Policy attachment
gets reviewed by the data-platform owner, who either renews the
attachment for another quarter or revokes it. The review
spreadsheet is generated from the IAM Policy graph and the
Production DB's own role catalog, so a principal that has been
granted access through a recent emergency change is impossible
to miss.

When a developer reports they cannot reach the Production DB, the
first check is whether the IAM Policy that authorized them last
quarter still exists. The IAM Policy restricts access by tag
selector; a principal that has been moved into a different team
group will lose its previous attachments and need a new policy
attached.
