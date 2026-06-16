import groovy.json.JsonOutput
import java.net.URLDecoder
import java.security.MessageDigest

definition(
    name: "Motional",
    namespace: "ejtbrown",
    author: "EJ Brown",
    description: "Expose selected motion and presence sensors through token-scoped HTTP endpoints.",
    category: "Convenience",
    iconUrl: "",
    iconX2Url: "",
    iconX3Url: "",
    oauth: true
)

preferences {
    page(name: "mainPage", title: "Motional", install: true, uninstall: true)
    page(name: "createTokenPage", title: "Create Motional token")
    page(name: "deleteTokenPage", title: "Delete Motional token")
}

mappings {
    path("/health") {
        action: [
            GET: "healthEndpoint",
            OPTIONS: "optionsEndpoint"
        ]
    }
    path("/:sensorName") {
        action: [
            GET: "sensorEndpoint",
            OPTIONS: "optionsEndpoint"
        ]
    }
}

def installed() {
    initialize()
}

def updated() {
    unsubscribe()
    initialize()
}

def initialize() {
    initializeState()

    if (motionSensors) {
        subscribe(motionSensors, "motion.active", activeEventHandler)
    }
    if (presenceSensors) {
        subscribe(presenceSensors, "presence.present", activeEventHandler)
    }

    seedCurrentlyActiveSensors()
    ensureHubitatAccessToken()
}

def mainPage() {
    initializeState()

    dynamicPage(name: "mainPage", title: "Motional", install: true, uninstall: true) {
        section("Sensors") {
            input "motionSensors", "capability.motionSensor",
                title: "Motion sensors",
                multiple: true,
                required: false,
                submitOnChange: true
            input "presenceSensors", "capability.presenceSensor",
                title: "Presence sensors",
                multiple: true,
                required: false,
                submitOnChange: true
            paragraph selectedSensorSummary()
        }

        section("Endpoint") {
            paragraph endpointSummary()
        }

        section("Tokens") {
            href(name: "createTokenHref",
                title: "Create token",
                page: "createTokenPage",
                description: "Create a token and grant it access to selected sensor API names.")
            href(name: "deleteTokenHref",
                title: "Delete token",
                page: "deleteTokenPage",
                description: "Revoke an existing Motional token.")
            paragraph tokenSummary()
        }

        if (state.lastCreatedTokenValue) {
            section("Latest created token") {
                paragraph "<pre>${htmlEscape(state.lastCreatedTokenValue)}</pre><p>Copy this now, then clear it. Authorization uses only the stored token hash.</p>"
                input "clearLatestTokenButton", "button",
                    title: "Clear latest token",
                    submitOnChange: true
            }
        }
    }
}

def createTokenPage() {
    initializeState()

    dynamicPage(name: "createTokenPage", title: "Create Motional token", install: false, uninstall: false) {
        section("Token") {
            input "newTokenLabel", "text",
                title: "Label",
                required: true,
                submitOnChange: true
            input "newTokenSensors", "enum",
                title: "Allowed sensor API names",
                options: sensorOptions(),
                multiple: true,
                required: true,
                submitOnChange: true
            input "createTokenButton", "button",
                title: "Create token",
                submitOnChange: true
        }
    }
}

def deleteTokenPage() {
    initializeState()

    dynamicPage(name: "deleteTokenPage", title: "Delete Motional token", install: false, uninstall: false) {
        section("Token") {
            input "deleteTokenHash", "enum",
                title: "Token",
                options: tokenDeleteOptions(),
                multiple: false,
                required: true,
                submitOnChange: true
            input "deleteTokenButton", "button",
                title: "Delete token",
                submitOnChange: true
        }
    }
}

def appButtonHandler(buttonName) {
    initializeState()

    if (buttonName == "createTokenButton") {
        createMotionalToken()
    } else if (buttonName == "deleteTokenButton") {
        deleteMotionalToken()
    } else if (buttonName == "clearLatestTokenButton") {
        state.lastCreatedTokenValue = null
    }
}

def healthEndpoint() {
    initializeState()

    renderJson(200, [
        ok: true,
        app: "Motional",
        sensors: sensorOptions().keySet().sort(),
        tokenCount: ((atomicState.tokens ?: [:]) as Map).size()
    ])
}

def optionsEndpoint() {
    render(status: 204, contentType: "application/json", data: "")
}

def sensorEndpoint() {
    initializeState()

    String requestedName = decodePathValue(params.sensorName)
    def sensor = findSensorByApiName(requestedName)
    if (!sensor) {
        renderJson(404, [
            error: "sensor_not_found",
            sensor: requestedName
        ])
        return
    }

    String apiName = apiNameForDevice(sensor)
    Map authorization = authorize(apiName)
    if (!authorization.ok) {
        renderJson(authorization.status as Integer, [
            error: authorization.error,
            sensor: apiName
        ])
        return
    }

    renderJson(200, sensorStatus(sensor, apiName))
}

private void initializeState() {
    if (!(atomicState.tokens instanceof Map)) {
        atomicState.tokens = [:]
    }
    if (!(atomicState.lastTriggered instanceof Map)) {
        atomicState.lastTriggered = [:]
    }
}

private void ensureHubitatAccessToken() {
    try {
        if (!state.accessToken) {
            createAccessToken()
        }
    } catch (ignored) {
        log.warn "Unable to create Hubitat app endpoint access token. Enable OAuth for this app in Hubitat if endpoint URLs require access_token."
    }
}

private void createMotionalToken() {
    List allowedSensors = normalizeList(newTokenSensors)
    if (!newTokenLabel || allowedSensors.isEmpty()) {
        log.warn "Token label and allowed sensors are required."
        return
    }

    String token = "${UUID.randomUUID().toString().replace('-', '')}${UUID.randomUUID().toString().replace('-', '')}"
    String hash = hashToken(token)
    Map tokens = ((atomicState.tokens ?: [:]) as Map).collectEntries { key, value -> [(key): value] }
    tokens[hash] = [
        label: newTokenLabel,
        sensors: allowedSensors,
        createdAt: isoDate(new Date()),
        prefix: token.substring(0, 8)
    ]
    atomicState.tokens = tokens
    state.lastCreatedTokenValue = token

    app.updateSetting("newTokenLabel", [type: "text", value: ""])
    app.updateSetting("newTokenSensors", [type: "enum", value: []])
}

private void deleteMotionalToken() {
    if (!deleteTokenHash) {
        return
    }

    Map tokens = ((atomicState.tokens ?: [:]) as Map).collectEntries { key, value -> [(key): value] }
    tokens.remove(deleteTokenHash as String)
    atomicState.tokens = tokens
    app.updateSetting("deleteTokenHash", [type: "enum", value: ""])
}

private Map authorize(String apiName) {
    String token = extractMotionalToken()
    if (!token) {
        return [ok: false, status: 401, error: "missing_bearer_token"]
    }

    Map tokenRecord = ((atomicState.tokens ?: [:]) as Map)[hashToken(token)] as Map
    if (!tokenRecord) {
        return [ok: false, status: 401, error: "invalid_bearer_token"]
    }

    List allowedSensors = normalizeList(tokenRecord.sensors)
    if (!allowedSensors.contains(apiName)) {
        return [ok: false, status: 403, error: "sensor_not_allowed"]
    }

    [ok: true, status: 200, token: tokenRecord]
}

private String extractMotionalToken() {
    String authorization = null

    try {
        authorization = request?.getHeader("Authorization")
    } catch (ignored) {
    }
    if (!authorization) {
        try {
            authorization = request?.headers?.Authorization ?: request?.headers?.authorization
        } catch (ignored) {
        }
    }

    if (authorization?.toLowerCase()?.startsWith("bearer ")) {
        return authorization.substring(7).trim()
    }

    params?.token ?: params?.motional_token
}

private Map sensorStatus(device, String apiName) {
    String attribute = null
    String value = null
    Boolean active = false

    String motion = safeCurrentValue(device, "motion")
    String presence = safeCurrentValue(device, "presence")

    if (motion != null) {
        attribute = "motion"
        value = motion
        active = motion == "active"
    } else if (presence != null) {
        attribute = "presence"
        value = presence
        active = presence == "present"
    } else {
        attribute = "unknown"
        value = "unknown"
        active = false
    }

    Date lastTriggered = lastTriggeredDate(device, apiName)
    if (active && !lastTriggered) {
        lastTriggered = new Date()
        recordLastTriggered(apiName, lastTriggered)
    }

    Long secondsSinceTriggered = null
    if (lastTriggered) {
        secondsSinceTriggered = Math.max(0L, ((new Date().time - lastTriggered.time) / 1000L) as Long)
    }
    if (active) {
        secondsSinceTriggered = 0L
    }

    [
        sensor: apiName,
        displayName: device.displayName,
        active: active,
        attribute: attribute,
        value: value,
        secondsSinceTriggered: secondsSinceTriggered,
        lastTriggeredAt: lastTriggered ? isoDate(lastTriggered) : null
    ]
}

private void activeEventHandler(evt) {
    def device = evt.device
    if (!device) {
        return
    }

    recordLastTriggered(apiNameForDevice(device), evt.date ?: new Date())
}

private void seedCurrentlyActiveSensors() {
    selectedDevices().each { device ->
        String motion = safeCurrentValue(device, "motion")
        String presence = safeCurrentValue(device, "presence")
        if (motion == "active" || presence == "present") {
            recordLastTriggered(apiNameForDevice(device), new Date())
        }
    }
}

private void recordLastTriggered(String apiName, Date date) {
    Map lastTriggered = ((atomicState.lastTriggered ?: [:]) as Map).collectEntries { key, value -> [(key): value] }
    lastTriggered[apiName] = isoDate(date)
    atomicState.lastTriggered = lastTriggered
}

private Date lastTriggeredDate(device, String apiName) {
    Date stored = parseIsoDate(((atomicState.lastTriggered ?: [:]) as Map)[apiName] as String)
    if (stored) {
        return stored
    }

    Date fromEvents = lastTriggeredFromEvents(device)
    if (fromEvents) {
        recordLastTriggered(apiName, fromEvents)
    }
    fromEvents
}

private Date lastTriggeredFromEvents(device) {
    try {
        def event = device.events(max: 100)?.find { evt ->
            (evt.name == "motion" && evt.value == "active") ||
                (evt.name == "presence" && evt.value == "present")
        }
        return event?.date
    } catch (ignored) {
        return null
    }
}

private String safeCurrentValue(device, String attribute) {
    try {
        def value = device.currentValue(attribute)
        return value == null ? null : value.toString()
    } catch (ignored) {
        return null
    }
}

private List selectedDevices() {
    Map byId = [:]
    normalizeList(motionSensors).each { device ->
        byId[device.id as String] = device
    }
    normalizeList(presenceSensors).each { device ->
        byId[device.id as String] = device
    }
    byId.values() as List
}

private Map sensorOptions() {
    selectedDevices().collectEntries { device ->
        String apiName = apiNameForDevice(device)
        [(apiName): "${apiName} (${device.displayName})"]
    }
}

private def findSensorByApiName(String apiName) {
    selectedDevices().find { device ->
        apiNameForDevice(device) == apiName || device.displayName == apiName
    }
}

private String apiNameForDevice(device) {
    Map namesById = apiNamesByDeviceId()
    namesById[device.id as String] ?: baseApiNameForDevice(device)
}

private Map apiNamesByDeviceId() {
    List devices = selectedDevices()
    Map baseCounts = [:].withDefault { 0 }
    devices.each { device ->
        baseCounts[baseApiNameForDevice(device)] = baseCounts[baseApiNameForDevice(device)] + 1
    }

    devices.collectEntries { device ->
        String baseName = baseApiNameForDevice(device)
        String apiName = baseCounts[baseName] > 1 ? "${baseName}-${device.id}" : baseName
        [(device.id as String): apiName]
    }
}

private String baseApiNameForDevice(device) {
    String rawName = (device.displayName ?: device.label ?: device.name ?: "sensor-${device.id}").toString()
    String apiName = rawName.toLowerCase()
        .replaceAll(/[^a-z0-9]+/, "-")
        .replaceAll(/(^-|-$)/, "")
    apiName ?: "sensor-${device.id}"
}

private String endpointSummary() {
    ensureHubitatAccessToken()

    String exampleSensor = sensorOptions().keySet().sort().with { it ? it[0] : "sensor-name" }
    String baseUrl = null
    try {
        baseUrl = getFullLocalApiServerUrl()?.replaceAll(/\/$/, "")
    } catch (ignored) {
        baseUrl = "http://<hubitat-host>/apps/api/<app-id>"
    }

    String endpoint = "${baseUrl}/${exampleSensor}"
    if (state.accessToken) {
        endpoint = "${endpoint}?access_token=${state.accessToken}"
    }

    "<p>Example URL:</p><pre>${htmlEscape(endpoint)}</pre><p>Send the Motional token as <code>Authorization: Bearer &lt;token&gt;</code>.</p>"
}

private String selectedSensorSummary() {
    Map options = sensorOptions()
    if (options.isEmpty()) {
        return "No sensors selected."
    }

    String rows = options.collect { key, label -> "${htmlEscape(key)} - ${htmlEscape(label)}" }.join("\n")
    "<pre>${rows}</pre>"
}

private String tokenSummary() {
    Map tokens = (atomicState.tokens ?: [:]) as Map
    if (tokens.isEmpty()) {
        return "No Motional tokens created."
    }

    String rows = tokens.collect { hash, record ->
        String label = record.label ?: "Unnamed token"
        String sensors = normalizeList(record.sensors).join(", ")
        "${htmlEscape(label)} (${htmlEscape(record.prefix ?: hash.take(8))}...) -> ${htmlEscape(sensors)}"
    }.join("\n")
    "<pre>${rows}</pre>"
}

private Map tokenDeleteOptions() {
    ((atomicState.tokens ?: [:]) as Map).collectEntries { hash, record ->
        String label = record.label ?: "Unnamed token"
        [(hash): "${label} (${record.prefix ?: hash.take(8)}...)"]
    }
}

private List normalizeList(value) {
    if (!value) {
        return []
    }
    if (value instanceof List) {
        return value.findAll { it != null }
    }
    if (value instanceof Collection) {
        return value.findAll { it != null } as List
    }
    [value]
}

private String hashToken(String token) {
    MessageDigest.getInstance("SHA-256")
        .digest(token.getBytes("UTF-8"))
        .encodeHex()
        .toString()
}

private String decodePathValue(value) {
    if (value == null) {
        return null
    }
    URLDecoder.decode(value.toString(), "UTF-8")
}

private String isoDate(Date date) {
    date.format("yyyy-MM-dd'T'HH:mm:ss'Z'", TimeZone.getTimeZone("UTC"))
}

private Date parseIsoDate(String value) {
    if (!value) {
        return null
    }
    try {
        return Date.parse("yyyy-MM-dd'T'HH:mm:ss'Z'", value)
    } catch (ignored) {
        return null
    }
}

private String htmlEscape(value) {
    value == null ? "" : value.toString()
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
}

private void renderJson(Integer statusCode, Map body) {
    render(status: statusCode, contentType: "application/json", data: JsonOutput.toJson(body))
}
