# Dashboard example source

The [dashboard namespace](dashboard/README.md) declares the union, composes
the three service calls, and exposes the UI handler.
The service files `session.holo`, `sql.holo`, and `cache.holo`
define public child families. Each service takes a copied scalar and returns a
scalar result. `ui.holo` maps the selected outcome to an observable code.
`main.holo` drives five scenarios. Check or run this source as the
[enclosing package](../README.md) so each file's `using` declarations resolve.
