import * as LiveView from "axum-live-view"

LiveView.connectAndRun({
  host: "localhost",
  // TODO: get port from query param
  port: 3000,
})
