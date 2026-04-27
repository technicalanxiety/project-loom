# Data Pipeline to Analytics Dashboard

The Data Pipeline produces every analytics event the platform emits.
Application services write structured events to a Kafka topic; the
Data Pipeline consumes them, normalizes the schema, drops
PII-flagged fields, and writes the results to the warehouse layer.

The Analytics Dashboard consumes the warehouse views the Data
Pipeline produces. There is no direct path from application services
to the Analytics Dashboard — every metric the dashboard renders has
been through the Data Pipeline first. That gives the data team a
single place to apply schema changes, retention policies, and
PII-scrubbing rules.

When a metric on the Analytics Dashboard looks wrong, the
debugging order is: (1) confirm the warehouse view is fresh, (2)
trace the row back through the Data Pipeline's transformation step,
(3) confirm the source application is still emitting the field on
its Kafka topic. The Data Pipeline produces a daily sanity-check
report that catches most schema drift before the Analytics Dashboard
shows users a wrong number.
