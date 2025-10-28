{{- if .Values.ingress.enabled }}
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: {{ include "rock-node.fullname" . }}
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
  {{- with .Values.ingress.annotations }}
  annotations:
{{ toYaml . | indent 4 }}
  {{- end }}
spec:
  {{- if .Values.ingress.className }}
  ingressClassName: {{ .Values.ingress.className }}
  {{- end }}
  rules:
  {{- range .Values.ingress.hosts }}
    - {{- if .host }}
      host: {{ .host }}
      {{- end }}
      http:
        paths:
        {{- range .paths }}
          - path: {{ .path }}
            pathType: {{ .pathType | default "Prefix" }}
            backend:
              service:
                {{- if eq .backend "grpc" }}
                name: {{ include "rock-node.fullname" $ }}
                port:
                  number: {{ $.Values.service.grpc.port }}
                {{- else if eq .backend "metrics" }}
                name: {{ include "rock-node.fullname" $ }}
                port:
                  number: {{ $.Values.service.metrics.port }}
                {{- else }}
                name: {{ .backend }}
                port:
                  number: {{ $.Values.service.grpc.port }}
                {{- end }}
        {{- end }}
  {{- end }}
  {{- if .Values.ingress.tls }}
  tls:
{{ toYaml .Values.ingress.tls | indent 4 }}
  {{- end }}
{{- end }}
