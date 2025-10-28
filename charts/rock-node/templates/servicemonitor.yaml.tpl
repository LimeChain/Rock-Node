{{- if and .Values.serviceMonitor.enabled .Values.service.metrics.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: {{ include "rock-node.fullname" . }}
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- with .Values.serviceMonitor.labels }}
{{ toYaml . | indent 2 }}
{{- end }}
{{- with .Values.serviceMonitor.annotations }}
  annotations:
{{ toYaml . | indent 4 }}
{{- end }}
spec:
  endpoints:
    - port: metrics
      path: /metrics
      interval: {{ .Values.serviceMonitor.interval }}
      scrapeTimeout: {{ .Values.serviceMonitor.scrapeTimeout }}
  selector:
    matchLabels:
      app.kubernetes.io/name: {{ include "rock-node.name" . }}
      app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
