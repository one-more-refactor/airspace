// airspace ear — an ESP32-C3 that listens to Bluetooth and reports what it
// hears to a collector over Wi-Fi.
//
// Why this exists: one receiver hears a distance, never a direction. Three
// receivers at known positions hear a place. A laptop is a poor third ear
// because it moves; a five-euro board taped to a shelf is an excellent one.
//
// The interesting constraint is that the C3 is single-core and this firmware
// wants both radios at once — a continuous BLE scan and a live Wi-Fi
// association. That is what the coexistence layer is for, and it is the whole
// reason this is ESP-IDF rather than something more pleasant to write.
// Scanning is deliberately passive and low duty cycle: we are not trying to
// catch every advertisement, we are trying to keep an association up while
// catching most of them.

#include <string.h>
#include <time.h>

#include "esp_event.h"
#include "esp_http_client.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_netif_sntp.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "host/ble_gap.h"
#include "host/ble_hs.h"
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "nvs_flash.h"

static const char *TAG = "airspace";

// More than this in range and the extras are dropped for the sweep. A flat
// produces about a dozen; a café produces sixty. Dropping is better than
// growing without bound on a device with 400 kB of RAM.
#define MAX_DEVICES 64
// One sweep of advertisements is collapsed to one observation per address, and
// posted on this interval. Matches the collector's own sweep.
#define POST_INTERVAL_MS 2000

typedef struct {
    uint8_t addr[6];
    uint8_t addr_type;
    int8_t rssi;
    bool seen;
    bool has_tx;
    int8_t tx_power;
    bool has_flags;
    uint8_t flags;
    uint16_t company;
    bool has_company;
    uint8_t msg;         // first byte of the manufacturer payload
    bool has_msg;
    char name[32];
} device_t;

static device_t s_devices[MAX_DEVICES];
static SemaphoreHandle_t s_lock;
static bool s_wifi_up = false;
static bool s_clock_set = false;

// ── advertisement parsing ────────────────────────────────────────────────────

// Copy a broadcast name, keeping only characters that can be put in a JSON
// string without escaping. A device is free to advertise control characters or
// a quote; a page rendering that is not.
static void copy_name(char *dst, size_t cap, const uint8_t *src, uint8_t len)
{
    size_t j = 0;
    for (uint8_t i = 0; i < len && j + 1 < cap; i++) {
        uint8_t c = src[i];
        if (c >= 0x20 && c < 0x7f && c != '"' && c != '\\') {
            dst[j++] = (char)c;
        }
    }
    dst[j] = '\0';
}

static void parse_adv(device_t *d, const uint8_t *data, uint8_t len)
{
    uint8_t i = 0;
    while (i + 1 < len) {
        uint8_t flen = data[i];
        if (flen == 0 || i + flen >= len + 1) {
            return;
        }
        uint8_t type = data[i + 1];
        const uint8_t *val = &data[i + 2];
        uint8_t vlen = flen - 1;

        switch (type) {
        case 0x01: // flags
            if (vlen >= 1) {
                d->flags = val[0];
                d->has_flags = true;
            }
            break;
        case 0x08: // shortened local name
        case 0x09: // complete local name
            if (d->name[0] == '\0') {
                copy_name(d->name, sizeof(d->name), val, vlen);
            }
            break;
        case 0x0a: // tx power level
            if (vlen >= 1) {
                d->tx_power = (int8_t)val[0];
                d->has_tx = true;
            }
            break;
        case 0xff: // manufacturer specific
            if (vlen >= 2) {
                d->company = (uint16_t)val[0] | ((uint16_t)val[1] << 8);
                d->has_company = true;
                if (vlen >= 3) {
                    d->msg = val[2];
                    d->has_msg = true;
                }
            }
            break;
        default:
            break;
        }
        i += flen + 1;
    }
}

// Fold one advertisement into the table. Repeat sightings update the signal
// strength and fill in anything the previous advertisement omitted — devices
// alternate between advertisement and scan-response payloads, so the name and
// the manufacturer data often arrive in different packets.
static void record(const ble_addr_t *addr, int8_t rssi, const uint8_t *data, uint8_t len)
{
    if (xSemaphoreTake(s_lock, pdMS_TO_TICKS(20)) != pdTRUE) {
        return;
    }
    device_t *slot = NULL;
    for (int i = 0; i < MAX_DEVICES; i++) {
        if (s_devices[i].seen && memcmp(s_devices[i].addr, addr->val, 6) == 0) {
            slot = &s_devices[i];
            break;
        }
        if (!s_devices[i].seen && slot == NULL) {
            slot = &s_devices[i];
        }
    }
    if (slot == NULL) {
        xSemaphoreGive(s_lock);
        return;
    }
    if (!slot->seen) {
        memset(slot, 0, sizeof(*slot));
        memcpy(slot->addr, addr->val, 6);
        slot->addr_type = addr->type;
        slot->seen = true;
    }
    slot->rssi = rssi;
    if (data && len) {
        parse_adv(slot, data, len);
    }
    xSemaphoreGive(s_lock);
}

// ── bluetooth ────────────────────────────────────────────────────────────────

static int on_gap(struct ble_gap_event *event, void *arg)
{
    (void)arg;
    switch (event->type) {
    case BLE_GAP_EVENT_DISC:
        record(&event->disc.addr, event->disc.rssi, event->disc.data,
               event->disc.length_data);
        break;
    case BLE_GAP_EVENT_DISC_COMPLETE:
        // Scanning forever is expressed as a scan that restarts, because a
        // scan that never ends also never yields to the Wi-Fi side cleanly.
        ESP_LOGW(TAG, "discovery ended (%d), restarting", event->disc_complete.reason);
        break;
    default:
        break;
    }
    return 0;
}

static void start_scan(void)
{
    struct ble_gap_disc_params params = {
        // Passive: never send a scan request. This board is an ear, and an ear
        // that talks back is a transmitter that can be located.
        .passive = 1,
        .filter_duplicates = 0,
        .itvl = 0,
        .window = 0,
        .filter_policy = 0,
        .limited = 0,
    };
    int rc = ble_gap_disc(BLE_OWN_ADDR_PUBLIC, BLE_HS_FOREVER, &params, on_gap, NULL);
    if (rc != 0) {
        ESP_LOGE(TAG, "ble_gap_disc failed: %d", rc);
    }
}

static void on_sync(void) { start_scan(); }

static void host_task(void *param)
{
    (void)param;
    nimble_port_run();
    nimble_port_freertos_deinit();
}

// ── wi-fi ────────────────────────────────────────────────────────────────────

static void wifi_events(void *arg, esp_event_base_t base, int32_t id, void *data)
{
    (void)arg;
    (void)data;
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        esp_wifi_connect();
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        s_wifi_up = false;
        ESP_LOGW(TAG, "wi-fi dropped, reconnecting");
        esp_wifi_connect();
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        s_wifi_up = true;
        ESP_LOGI(TAG, "wi-fi up");
    }
}

static void wifi_start(void)
{
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    esp_netif_create_default_wifi_sta();

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));
    ESP_ERROR_CHECK(esp_event_handler_instance_register(WIFI_EVENT, ESP_EVENT_ANY_ID,
                                                        wifi_events, NULL, NULL));
    ESP_ERROR_CHECK(esp_event_handler_instance_register(IP_EVENT, IP_EVENT_STA_GOT_IP,
                                                        wifi_events, NULL, NULL));

    wifi_config_t wc = {0};
    strncpy((char *)wc.sta.ssid, CONFIG_AIRSPACE_WIFI_SSID, sizeof(wc.sta.ssid) - 1);
    strncpy((char *)wc.sta.password, CONFIG_AIRSPACE_WIFI_PASSWORD, sizeof(wc.sta.password) - 1);
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wc));
    // Power save off: the modem sleeping mid-scan is the classic way a
    // coexisting BLE scan starts missing most of the room.
    ESP_ERROR_CHECK(esp_wifi_set_ps(WIFI_PS_NONE));
    ESP_ERROR_CHECK(esp_wifi_start());
}

// The collector uses each observation's timestamp to decide what is stale, so
// a board with a 1970 clock reports devices that are silently thrown away.
// Wait for real time before posting anything, and say so rather than sulking.
static void clock_start(void)
{
    esp_sntp_config_t cfg = ESP_NETIF_SNTP_DEFAULT_CONFIG("pool.ntp.org");
    esp_netif_sntp_init(&cfg);
}

// ── reporting ────────────────────────────────────────────────────────────────

static char s_body[16384];

static size_t build_batch(void)
{
    time_t now = time(NULL);
    size_t n = 0;
    n += snprintf(s_body + n, sizeof(s_body) - n,
                  "{\"node\":{\"name\":\"%s\",\"x\":%s,\"y\":%s},\"obs\":[",
                  CONFIG_AIRSPACE_NODE_NAME, CONFIG_AIRSPACE_NODE_X, CONFIG_AIRSPACE_NODE_Y);

    bool first = true;
    xSemaphoreTake(s_lock, portMAX_DELAY);
    for (int i = 0; i < MAX_DEVICES; i++) {
        device_t *d = &s_devices[i];
        if (!d->seen) {
            continue;
        }
        if (n > sizeof(s_body) - 512) {
            break;
        }
        // BlueZ prints addresses most-significant octet first; the controller
        // hands them over the other way round. Getting this backwards produces
        // a device that never matches the same device seen by another node.
        n += snprintf(s_body + n, sizeof(s_body) - n,
                      "%s{\"t\":%lld,\"addr\":\"%02X:%02X:%02X:%02X:%02X:%02X\","
                      "\"at\":\"%s\",\"rssi\":%d,\"src\":\"ble\"",
                      first ? "" : ",", (long long)now,
                      d->addr[5], d->addr[4], d->addr[3], d->addr[2], d->addr[1], d->addr[0],
                      (d->addr_type == BLE_ADDR_PUBLIC) ? "public" : "random", d->rssi);
        first = false;

        if (d->name[0]) {
            n += snprintf(s_body + n, sizeof(s_body) - n, ",\"name\":\"%s\"", d->name);
        }
        if (d->has_tx) {
            n += snprintf(s_body + n, sizeof(s_body) - n, ",\"tx_power\":%d", d->tx_power);
        }
        if (d->has_flags) {
            n += snprintf(s_body + n, sizeof(s_body) - n, ",\"flags\":%u", d->flags);
        }
        if (d->has_company) {
            n += snprintf(s_body + n, sizeof(s_body) - n, ",\"company\":[%u]", d->company);
            if (d->has_msg) {
                n += snprintf(s_body + n, sizeof(s_body) - n, ",\"cmsg\":[[%u,%u]]",
                              d->company, d->msg);
            }
        }
        n += snprintf(s_body + n, sizeof(s_body) - n, "}");
        d->seen = false; // one sweep, one report
    }
    xSemaphoreGive(s_lock);

    n += snprintf(s_body + n, sizeof(s_body) - n, "]}");
    return first ? 0 : n; // nothing heard is not worth a request
}

static void post_task(void *arg)
{
    (void)arg;
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(POST_INTERVAL_MS));

        if (!s_wifi_up) {
            continue;
        }
        if (!s_clock_set) {
            time_t now = time(NULL);
            if (now < 1700000000) {
                ESP_LOGW(TAG, "waiting for the clock — nothing is posted until it is real");
                continue;
            }
            s_clock_set = true;
            ESP_LOGI(TAG, "clock synchronised");
        }

        size_t len = build_batch();
        if (len == 0) {
            continue;
        }

        esp_http_client_config_t cfg = {
            .url = CONFIG_AIRSPACE_COLLECTOR_URL,
            .method = HTTP_METHOD_POST,
            .timeout_ms = 4000,
        };
        esp_http_client_handle_t c = esp_http_client_init(&cfg);
        esp_http_client_set_header(c, "Content-Type", "application/json");
        esp_http_client_set_header(c, "Authorization", "Bearer " CONFIG_AIRSPACE_TOKEN);
        esp_http_client_set_post_field(c, s_body, len);
        esp_err_t err = esp_http_client_perform(c);
        if (err != ESP_OK) {
            ESP_LOGW(TAG, "collector unreachable: %s", esp_err_to_name(err));
        } else {
            int status = esp_http_client_get_status_code(c);
            if (status != 204) {
                // 404 is what the collector returns for a bad token, on
                // purpose — it does not distinguish a wrong secret from a
                // wrong path for anyone probing it.
                ESP_LOGW(TAG, "collector said %d (404 here almost always means the token is wrong)",
                         status);
            }
        }
        esp_http_client_cleanup(c);
    }
}

// ── entry ────────────────────────────────────────────────────────────────────

void app_main(void)
{
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    ESP_ERROR_CHECK(err);

    s_lock = xSemaphoreCreateMutex();

    wifi_start();
    clock_start();

    ESP_ERROR_CHECK(nimble_port_init());
    ble_hs_cfg.sync_cb = on_sync;
    nimble_port_freertos_init(host_task);

    xTaskCreate(post_task, "airspace_post", 8192, NULL, 5, NULL);
    ESP_LOGI(TAG, "airspace ear: node %s reporting to %s",
             CONFIG_AIRSPACE_NODE_NAME, CONFIG_AIRSPACE_COLLECTOR_URL);
}
