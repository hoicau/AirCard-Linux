/* Minimal Linux-only adapter. Uses installed public headers, never Apple binaries. */
#include <libimobiledevice/libimobiledevice.h>
#include <libimobiledevice/lockdown.h>
#include <libimobiledevice/service.h>
#include <libimobiledevice/afc.h>
#include <plist/plist.h>
#include <usbmuxd.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <sys/socket.h>
#include <sys/time.h>

typedef struct { int domain; int code; } ac_error;
typedef struct { char udid[128]; int transport; } ac_device;
typedef struct { idevice_t device; lockdownd_client_t lockdown; } ac_session;
static int fail(ac_error *e, int domain, int code) { e->domain = domain; e->code = code; return -1; }
static void wipe(void *p, size_t n) { volatile unsigned char *b = p; while (n--) *b++ = 0; }
void ac_free(void *p) { free(p); }

int ac_list(ac_device **out, int *count, ac_error *e) {
    idevice_info_t *list = NULL;
    int n = 0;
    int r = idevice_get_device_list_extended(&list, &n);
    if (r != 0) return fail(e, 5, r);
    if (n < 0 || n > 4096) { idevice_device_list_extended_free(list); return fail(e, 6, -1); }
    ac_device *rows = calloc((size_t)n + 1, sizeof(*rows));
    if (!rows) { idevice_device_list_extended_free(list); return fail(e, 6, -2); }
    for (int i = 0; i < n; i++) {
        if (!list[i] || !list[i]->udid || strlen(list[i]->udid) >= sizeof(rows[i].udid)) {
            free(rows); idevice_device_list_extended_free(list); return fail(e, 6, -1);
        }
        strcpy(rows[i].udid, list[i]->udid);
        rows[i].transport = (int)list[i]->conn_type;
    }
    idevice_device_list_extended_free(list);
    *out = rows; *count = n;
    return 0;
}

void ac_close(ac_session *s) {
    if (!s) return;
    if (s->lockdown) lockdownd_client_free(s->lockdown);
    if (s->device) idevice_free(s->device);
    free(s);
}

ac_session *ac_open(const char *udid, int transport, ac_error *e) {
    char *record = NULL, *host_id = NULL;
    uint32_t size = 0;
    plist_t pair = NULL;
    int r = usbmuxd_read_pair_record(udid, &record, &size);
    if (r != 0 || !record || size == 0 || size > 1024 * 1024) {
        if (record) { wipe(record, size); free(record); }
        fail(e, r == -ENOENT || r == 0 ? 2 : 5, r == -ENOENT || r == 0 ? -29 : r);
        return NULL;
    }
    /* Only HostID leaves this temporary pair-record scope. Never log any record. */
    if (size >= 8 && memcmp(record, "bplist00", 8) == 0) plist_from_bin(record, size, &pair);
    else plist_from_xml(record, size, &pair);
    wipe(record, size); free(record);
    if (pair) {
        plist_t node = plist_dict_get_item(pair, "HostID");
        if (node && plist_get_node_type(node) == PLIST_STRING) plist_get_string_val(node, &host_id);
        plist_free(pair);
    }
    if (!host_id) { fail(e, 2, -31); return NULL; }
    ac_session *s = calloc(1, sizeof(*s));
    if (!s) { wipe(host_id, strlen(host_id)); free(host_id); fail(e, 6, -2); return NULL; }
    enum idevice_options options = transport == 1 ? IDEVICE_LOOKUP_USBMUX : IDEVICE_LOOKUP_NETWORK;
    r = idevice_new_with_options(&s->device, udid, options);
    int domain = 1;
    if (!r) {
        domain = 2;
        r = lockdownd_client_new(s->device, &s->lockdown, "AirCard-Linux");
        /* Explicit existing-host session: unlike new_with_handshake, never calls Pair. */
        if (!r) r = lockdownd_start_session(s->lockdown, host_id, NULL, NULL);
    }
    wipe(host_id, strlen(host_id)); free(host_id);
    if (r) { fail(e, domain, r); ac_close(s); return NULL; }
    return s;
}

int ac_version(ac_session *s, char *out, uint32_t capacity, ac_error *e) {
    plist_t value = NULL;
    int r = lockdownd_get_value(s->lockdown, NULL, "ProductVersion", &value);
    if (r) { if (value) plist_free(value); return fail(e, 2, r); }
    char *str = NULL;
    if (value && plist_get_node_type(value) == PLIST_STRING) plist_get_string_val(value, &str);
    if (value) plist_free(value);
    if (!str || strlen(str) >= capacity) { free(str); return fail(e, 6, -1); }
    strcpy(out, str); free(str); return 0;
}

void *ac_start(ac_session *s, const char *name, int afc, int *tls, ac_error *e) {
    lockdownd_service_descriptor_t descriptor = NULL;
    int r = lockdownd_start_service(s->lockdown, name, &descriptor);
    if (r) { if (descriptor) lockdownd_service_descriptor_free(descriptor); fail(e, 2, r); return NULL; }
    if (!descriptor || !descriptor->port) { if (descriptor) lockdownd_service_descriptor_free(descriptor); fail(e, 6, -1); return NULL; }
    *tls = descriptor->ssl_enabled != 0;
    void *client = NULL;
    /* service_client_new in libimobiledevice 1.4.0 ignores TLS-enable errors.
       Use public idevice connection APIs and check TLS explicitly for raw services.
       AFC has no public connection-injection API: fail closed if it requests TLS. */
    int domain = afc ? 4 : 1;
    if (afc) {
        if (*tls) { lockdownd_service_descriptor_free(descriptor); fail(e, 3, -4); return NULL; }
        r = afc_client_new(s->device, descriptor, (afc_client_t *)&client);
    } else {
        r = idevice_connect(s->device, descriptor->port, (idevice_connection_t *)&client);
        if (!r && *tls) {
            r = idevice_connection_enable_ssl(client);
            if (r) { idevice_disconnect(client); client = NULL; }
        }
    }
    lockdownd_service_descriptor_free(descriptor);
    if (r) { fail(e, domain, r); return NULL; }
    return client;
}
void ac_service_free(void *p) { idevice_disconnect(p); }
int ac_receive(void *p, char *buf, uint32_t size, uint32_t *received, uint32_t timeout, ac_error *e) {
    int r = idevice_connection_receive_timeout(p, buf, size, received, timeout);
    /* Preserve partial data before reporting a later timeout. */
    if (*received) return 0;
    return r ? fail(e, 1, r) : 0;
}
int ac_send(void *p, const char *buf, uint32_t size, uint32_t *sent, uint32_t timeout, ac_error *e) {
    int fd = -1;
    int r = idevice_connection_get_fd(p, &fd);
    if (r) return fail(e, 1, r);
    struct timeval tv = { (time_t)(timeout / 1000), (suseconds_t)((timeout % 1000) * 1000) };
    if (!timeout || setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) != 0)
        return fail(e, 6, -1);
    r = idevice_connection_send(p, buf, size, sent);
    return r ? fail(e, 1, r) : 0;
}
void ac_afc_free(void *p) { afc_client_free(p); }
void ac_dictionary_free(char **p) { afc_dictionary_free(p); }
int ac_afc_list(void *p, const char *path, char ***out, ac_error *e) {
    int r = afc_read_directory(p, path, out); return r ? fail(e, 4, r) : 0;
}
int ac_afc_info(void *p, const char *path, char ***out, ac_error *e) {
    int r = afc_get_file_info(p, path, out); return r ? fail(e, 4, r) : 0;
}
int ac_afc_mkdir(void *p, const char *path, ac_error *e) {
    int r = afc_make_directory(p, path); return r ? fail(e, 4, r) : 0;
}
int ac_afc_remove(void *p, const char *path, ac_error *e) {
    int r = afc_remove_path(p, path); return r ? fail(e, 4, r) : 0;
}
int ac_afc_rename(void *p, const char *source, const char *target, ac_error *e) {
    int r = afc_rename_path(p, source, target); return r ? fail(e, 4, r) : 0;
}
int ac_afc_open(void *p, const char *path, int write, uint64_t *handle, ac_error *e) {
    int r = afc_file_open(p, path, write ? AFC_FOPEN_WRONLY : AFC_FOPEN_RDONLY, handle);
    return r ? fail(e, 4, r) : 0;
}
int ac_afc_file_close(void *p, uint64_t handle, ac_error *e) {
    int r = afc_file_close(p, handle); return r ? fail(e, 4, r) : 0;
}
int ac_afc_read(void *p, uint64_t handle, char *buf, uint32_t size, uint32_t *read, ac_error *e) {
    int r = afc_file_read(p, handle, buf, size, read); return r ? fail(e, 4, r) : 0;
}
int ac_afc_write(void *p, uint64_t handle, const char *buf, uint32_t size, uint32_t *written, ac_error *e) {
    int r = afc_file_write(p, handle, buf, size, written); return r ? fail(e, 4, r) : 0;
}
