/* Shared fixture used through the C header by Rust and Python boundary tests. */
#include "ga4gh_extension.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)
static void pause_ms(void) { Sleep(2); }
#else
#include <unistd.h>
#define EXPORT __attribute__((visibility("default")))
static void pause_ms(void) { usleep(2000); }
#endif
#ifndef PLUGIN_NAME
#define PLUGIN_NAME "fixture"
#endif
#ifndef PLUGIN_VERSION
#define PLUGIN_VERSION "1.0.0"
#endif
#ifndef TABLE_ABI
#define TABLE_ABI 1
#endif
#ifndef TABLE_SIZE
#define TABLE_SIZE sizeof(Ga4ghExtensionApiV1)
#endif
#ifndef TARGET_TRIPLE
#error Define TARGET_TRIPLE to the host Rust target triple
#endif
static const char descriptor[] = "{\"name\":\"" PLUGIN_NAME "\",\"version\":\"" PLUGIN_VERSION
    "\",\"target\":\"" TARGET_TRIPLE "\",\"abi_version\":1,\"capabilities\":{\"test\":[1,2]}}";
typedef struct { unsigned char *config; size_t len; size_t pending; char drop_path[2048]; } Instance;
static Ga4ghOwnedBytes copy(const unsigned char *data,size_t len) {
    Ga4ghOwnedBytes out={NULL,len};
    if(len) { out.data=(unsigned char*)malloc(len); if(!out.data) abort(); memcpy(out.data,data,len); }
    return out;
}
static int equals(Ga4ghBytes bytes,const char *text) {return bytes.len==strlen(text) && (!bytes.len || memcmp(bytes.data,text,bytes.len)==0);}
static void path_string(char *out,size_t cap,Ga4ghBytes input,const char *suffix) {
    size_t len=input.len; if(len+strlen(suffix)>=cap) abort();
    if(len) memcpy(out,input.data,len); strcpy(out+len,suffix);
}
static void touch(const char *path,const char *text) { FILE *f=fopen(path,"wb");if(!f) abort();fputs(text,f);fclose(f); }
static int exists(const char *path) {FILE *f=fopen(path,"rb");if(!f)return 0;fclose(f);return 1;}
static int32_t create_instance(Ga4ghBytes config,void **out,Ga4ghOwnedBytes *error) {
    if(config.len>=4 && memcmp(config.data,"fail",4)==0) {*error=copy(config.data,config.len);return 7;}
    Instance *instance=(Instance*)calloc(1,sizeof(Instance));if(!instance)abort();
    Ga4ghOwnedBytes copied=copy(config.data,config.len);instance->config=copied.data;instance->len=copied.len;*out=instance;return 0;
}
static int32_t invoke(void *ptr,Ga4ghBytes capability,uint32_t revision,Ga4ghBytes operation,Ga4ghBytes input,Ga4ghOwnedBytes *out) {
    Instance *instance=(Instance*)ptr;int32_t status=0;
    if(!equals(capability,"test") || (revision!=1 && revision!=2)) return 9;
    if(equals(operation,"configuration")) {*out=copy(instance->config,instance->len);}
    else if(equals(operation,"set_drop_path")) {path_string(instance->drop_path,sizeof(instance->drop_path),input,"");}
    else if(equals(operation,"wait")) {
        char entered[2048],release[2048];path_string(entered,sizeof(entered),input,".entered");path_string(release,sizeof(release),input,".release");
        touch(entered,"entered");time_t deadline=time(NULL)+15;
        while(!exists(release)) {if(time(NULL)>deadline)return 10;pause_ms();}
        *out=copy(input.data,input.len);
    }
    else if(equals(operation,"touch")) {char path[2048];path_string(path,sizeof(path),input,"");touch(path,"touched");}
    else { *out=copy(input.data,input.len);if(equals(operation,"fail"))status=8; }
    if(out->data) instance->pending++;
    return status;
}
static void destroy_instance(void *ptr) {Instance *instance=(Instance*)ptr;if(instance->drop_path[0])touch(instance->drop_path,instance->pending?"unreleased":"destroyed");free(instance->config);free(instance);}
static void free_buffer(void *ptr,Ga4ghOwnedBytes buffer) {if(ptr && buffer.data)((Instance*)ptr)->pending--;free(buffer.data);}
/* Extra bytes exercise compatible extension of the API table. */
static const struct {Ga4ghExtensionApiV1 api; uint64_t future_field;} table={
    {{TABLE_ABI,TABLE_SIZE},{(const uint8_t*)descriptor,sizeof(descriptor)-1},create_instance,invoke,destroy_instance,free_buffer},42
};
EXPORT const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void) {
#ifdef ENTRY_WAIT_PREFIX
    touch(ENTRY_WAIT_PREFIX ".entered","entered");time_t deadline=time(NULL)+15;
    while(!exists(ENTRY_WAIT_PREFIX ".release")) {if(time(NULL)>deadline)abort();pause_ms();}
#endif
    return &table.api;
}
