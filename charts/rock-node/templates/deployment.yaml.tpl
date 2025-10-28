apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "rock-node.fullname" . }}
  namespace: {{ include "rock-node.namespace" . }}
  labels:
    app.kubernetes.io/name: {{ include "rock-node.name" . }}
    helm.sh/chart: {{ include "rock-node.chart" . }}
    app.kubernetes.io/instance: {{ .Release.Name }}
    app.kubernetes.io/managed-by: {{ .Release.Service }}
spec:
  replicas: {{ .Values.replicaCount }}
  selector:
    matchLabels:
      app.kubernetes.io/name: {{ include "rock-node.name" . }}
      app.kubernetes.io/instance: {{ .Release.Name }}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: {{ include "rock-node.name" . }}
        app.kubernetes.io/instance: {{ .Release.Name }}
      {{- with .Values.podLabels }}
{{ toYaml . | indent 6 }}
      {{- end }}
      {{- with .Values.podAnnotations }}
      annotations:
{{ toYaml . | indent 8 }}
      {{- end }}
    spec:
      serviceAccountName: {{ include "rock-node.serviceAccountName" . }}
      {{- if .Values.imagePullSecrets }}
      imagePullSecrets:
{{ toYaml .Values.imagePullSecrets | indent 8 }}
      {{- end }}
      {{- if and .Values.securityContext.enabled }}
      securityContext:
        fsGroup: {{ .Values.securityContext.fsGroup }}
        runAsUser: {{ .Values.securityContext.runAsUser }}
        runAsGroup: {{ .Values.securityContext.runAsGroup }}
        runAsNonRoot: {{ .Values.securityContext.runAsNonRoot }}
        {{- if .Values.securityContext.supplementalGroups }}
        supplementalGroups:
{{ toYaml .Values.securityContext.supplementalGroups | indent 10 }}
        {{- end }}
      {{- end }}
      containers:
        - name: rock-node
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag }}"
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          {{- if and .Values.containerSecurityContext.enabled }}
          securityContext:
            allowPrivilegeEscalation: {{ .Values.containerSecurityContext.allowPrivilegeEscalation }}
            capabilities:
{{ toYaml .Values.containerSecurityContext.capabilities | indent 14 }}
          {{- end }}
          env:
            {{- if .Values.extraEnv }}
{{ toYaml .Values.extraEnv | indent 12 }}
            {{- else }}
            []
            {{- end }}
          ports:
            - name: grpc
              containerPort: {{ .Values.service.grpc.targetPort }}
              protocol: TCP
            {{- if .Values.service.metrics.enabled }}
            - name: metrics
              containerPort: {{ .Values.service.metrics.targetPort }}
              protocol: TCP
            {{- end }}
          {{- if .Values.probes.liveness.enabled }}
          livenessProbe:
            httpGet:
              path: {{ .Values.probes.liveness.path }}
              port: {{ .Values.probes.liveness.port }}
            initialDelaySeconds: {{ .Values.probes.liveness.initialDelaySeconds }}
            periodSeconds: {{ .Values.probes.liveness.periodSeconds }}
          {{- end }}
          {{- if .Values.probes.readiness.enabled }}
          readinessProbe:
            httpGet:
              path: {{ .Values.probes.readiness.path }}
              port: {{ .Values.probes.readiness.port }}
            initialDelaySeconds: {{ .Values.probes.readiness.initialDelaySeconds }}
            periodSeconds: {{ .Values.probes.readiness.periodSeconds }}
          {{- end }}
          resources:
{{ toYaml .Values.resources | indent 12 }}
          volumeMounts:
            - name: config
              mountPath: /config
              readOnly: true
            {{- if .Values.persistence.enabled }}
            - name: data
              mountPath: /app/data
            {{- end }}
            {{- with .Values.extraVolumeMounts }}
{{ toYaml . | indent 12 }}
            {{- end }}
      volumes:
        - name: config
          {{- if .Values.config.existingConfigMap }}
          configMap:
            name: {{ .Values.config.existingConfigMap }}
          {{- else }}
          configMap:
            name: {{ include "rock-node.fullname" . }}-config
          {{- end }}
        {{- if .Values.persistence.enabled }}
        - name: data
          persistentVolumeClaim:
            claimName: {{ if .Values.persistence.existingClaim }}{{ .Values.persistence.existingClaim }}{{ else }}{{ include "rock-node.fullname" . }}-data{{ end }}
        {{- end }}
        {{- with .Values.extraVolumes }}
{{ toYaml . | indent 8 }}
        {{- end }}
      {{- with .Values.nodeSelector }}
      nodeSelector:
{{ toYaml . | indent 8 }}
      {{- end }}
      {{- with .Values.tolerations }}
      tolerations:
{{ toYaml . | indent 8 }}
      {{- end }}
      {{- with .Values.affinity }}
      affinity:
{{ toYaml . | indent 8 }}
      {{- end }}
