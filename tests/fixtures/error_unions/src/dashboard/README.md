# Dashboard namespace

`customer_count.holo` imports the three services, declares their error
families as children, and uses `try` to compose their calls.
`dashboard.holo` imports the UI and handles the result from the private
loader in the same namespace. Each file has its own `using` directives.
Run both contributors through the [dashboard package](../../README.md).
